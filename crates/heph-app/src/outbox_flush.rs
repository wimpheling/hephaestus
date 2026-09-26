use super::{AppError, Arc, Duration, Future, Instant, StdMutex, component};

#[derive(Debug, Clone, Copy)]
pub enum FlushPublisher {
    Forge,
    Release,
    Review,
    ProductEvent,
    Mailbox,
}

impl FlushPublisher {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Forge => "forge",
            Self::Release => "release",
            Self::Review => "review",
            Self::ProductEvent => "product-event",
            Self::Mailbox => "mailbox",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Forge => 0,
            Self::Release => 1,
            Self::Review => 2,
            Self::ProductEvent => 3,
            Self::Mailbox => 4,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FlushPhase {
    pub publisher: FlushPublisher,
    pub started: Instant,
    pub elapsed: Duration,
    pub batch_count: Option<usize>,
}

#[derive(Debug)]
pub struct FlushDiagnostics {
    pub entered_remaining: Duration,
    pub passes: u32,
    pub active: Option<FlushPhase>,
    pub last_phase: Option<FlushPhase>,
    pub last_batches: [Option<usize>; 5],
    pub failure_kind: Option<&'static str>,
    pub deadline_expired_before_pass: bool,
    pub deadline_expired_during_pass: bool,
    pub deadline_expired_during_publisher: bool,
}

impl FlushDiagnostics {
    pub fn new(deadline: Instant) -> Self {
        Self {
            entered_remaining: deadline.saturating_duration_since(Instant::now()),
            passes: 0,
            active: None,
            last_phase: None,
            last_batches: [None; 5],
            failure_kind: None,
            deadline_expired_before_pass: false,
            deadline_expired_during_pass: false,
            deadline_expired_during_publisher: false,
        }
    }

    pub fn begin(&mut self, publisher: FlushPublisher) {
        self.active = Some(FlushPhase {
            publisher,
            started: Instant::now(),
            elapsed: Duration::ZERO,
            batch_count: None,
        });
    }

    pub fn finish(&mut self, batch_count: usize) {
        if let Some(mut phase) = self.active.take() {
            phase.elapsed = phase.started.elapsed();
            phase.batch_count = Some(batch_count);
            self.last_batches[phase.publisher.index()] = Some(batch_count);
            self.last_phase = Some(phase);
        }
    }

    pub fn finish_error(&mut self) {
        if let Some(mut phase) = self.active.take() {
            phase.elapsed = phase.started.elapsed();
            self.last_phase = Some(phase);
        }
    }

    pub const fn set_deadline_before_pass(&mut self) {
        self.failure_kind = Some("deadline-before-pass");
        self.deadline_expired_before_pass = true;
    }

    pub fn log_failure(&self, deadline: Instant) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let active_publisher = self.active.map(|phase| phase.publisher.name());
        let active_elapsed = self
            .active
            .map(|phase| duration_millis(phase.started.elapsed()));
        let last_publisher = self.last_phase.map(|phase| phase.publisher.name());
        let last_elapsed = self.last_phase.map(|phase| duration_millis(phase.elapsed));
        tracing::warn!(
            entered_remaining_ms = duration_millis(self.entered_remaining),
            remaining_ms = duration_millis(remaining),
            passes = self.passes,
            deadline_expired_before_pass = self.deadline_expired_before_pass,
            deadline_expired_during_pass = self.deadline_expired_during_pass,
            deadline_expired_during_publisher = self.deadline_expired_during_publisher,
            active_publisher = active_publisher.unwrap_or("none"),
            active_elapsed_ms = active_elapsed.unwrap_or(0),
            last_publisher = last_publisher.unwrap_or("none"),
            last_elapsed_ms = last_elapsed.unwrap_or(0),
            forge_last_batch = ?self.last_batches[FlushPublisher::Forge.index()],
            release_last_batch = ?self.last_batches[FlushPublisher::Release.index()],
            review_last_batch = ?self.last_batches[FlushPublisher::Review.index()],
            product_event_last_batch = ?self.last_batches[FlushPublisher::ProductEvent.index()],
            mailbox_last_batch = ?self.last_batches[FlushPublisher::Mailbox.index()],
            failure_kind = self.failure_kind.unwrap_or("unknown"),
            "final outbox flush did not quiesce"
        );
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub async fn flush_publisher<Fut, Error>(
    diagnostics: &Arc<StdMutex<FlushDiagnostics>>,
    publisher: FlushPublisher,
    operation: Fut,
    component_name: &'static str,
) -> Result<usize, AppError>
where
    Fut: Future<Output = Result<usize, Error>>,
    Error: std::fmt::Display,
{
    diagnostics
        .lock()
        .expect("flush diagnostics mutex is not poisoned")
        .begin(publisher);
    match operation.await {
        Ok(batch_count) => {
            diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned")
                .finish(batch_count);
            Ok(batch_count)
        }
        Err(error) => {
            let mut diagnostics = diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned");
            diagnostics.failure_kind = Some("publisher-error");
            diagnostics.finish_error();
            drop(diagnostics);
            Err(component(component_name)(error))
        }
    }
}

pub async fn flush_until_quiescent<F, Fut>(
    deadline: Instant,
    diagnostics: Arc<StdMutex<FlushDiagnostics>>,
    mut pass: F,
) -> Result<(), AppError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool, AppError>>,
{
    loop {
        {
            let mut diagnostics = diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned");
            diagnostics.passes = diagnostics.passes.saturating_add(1);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned")
                .set_deadline_before_pass();
            return Err(AppError::Shutdown(String::from(
                "final outbox flush did not quiesce",
            )));
        }
        let quiescent = tokio::time::timeout(remaining, pass())
            .await
            .map_err(|_| {
                let mut diagnostics = diagnostics
                    .lock()
                    .expect("flush diagnostics mutex is not poisoned");
                diagnostics.failure_kind = Some(if diagnostics.active.is_some() {
                    "deadline-during-publisher"
                } else {
                    "deadline-during-pass"
                });
                diagnostics.deadline_expired_during_pass = true;
                diagnostics.deadline_expired_during_publisher = diagnostics.active.is_some();
                drop(diagnostics);
                AppError::Shutdown(String::from("final outbox flush did not quiesce"))
            })??;
        if quiescent && Instant::now() < deadline {
            return Ok(());
        }
    }
}
