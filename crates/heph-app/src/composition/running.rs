use super::{
    AppError, Arc, CancelRun, CommandId, ControlPlanePool, Duration, FlushDiagnostics,
    FlushPublisher, ForgeNatsOutboxPublisher, MailboxOutboxPublisher, PgForgeRepository, PgPool,
    PgRunRepository, PostgresMailboxRepository, PostgresReviewRepository, ReleaseOutboxPublisher,
    ReviewOutboxPublisher, RunEventKind, RunId, RunOrchestrator, RunRepository, StdMutex,
    component, event_adapter, flush_publisher, flush_until_quiescent,
};
use std::{net::SocketAddr, time::Instant};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Running daemon handle.
pub struct RunningHephaestus {
    pub(super) http_addr: SocketAddr,
    pub(super) cancellation: CancellationToken,
    pub(super) tasks: Vec<JoinHandle<Result<(), String>>>,
    pub(super) pool: PgPool,
    pub(super) application_pool: PgPool,
    pub(super) service_log_pool: PgPool,
    pub(super) nats_client: async_nats::Client,
    pub(super) jetstream: async_nats::jetstream::Context,
    pub(super) forge: Arc<PgForgeRepository>,
    pub(super) run_repository: Arc<PgRunRepository>,
    pub(super) mailbox_repository: Arc<PostgresMailboxRepository>,
    pub(super) review_repository: Arc<PostgresReviewRepository>,
    pub(super) orchestrator: Arc<RunOrchestrator>,
    pub(super) outbox_batch_size: i64,
    pub(super) product_event_cursor_key: [u8; 32],
    pub(super) shutdown_timeout: Duration,
}

impl RunningHephaestus {
    /// Bound HTTP address after the readiness barrier.
    #[must_use]
    pub const fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    /// Returns a retained application-role pool for daemon integration checks.
    #[cfg(feature = "test-fixtures")]
    #[doc(hidden)]
    #[must_use]
    pub fn application_pool_for_test(&self) -> ControlPlanePool {
        self.application_pool.clone()
    }

    /// Returns the dedicated worker pool used by the service-log scheduler.
    #[cfg(feature = "test-fixtures")]
    #[doc(hidden)]
    #[must_use]
    pub fn service_log_pool_for_test(&self) -> ControlPlanePool {
        self.service_log_pool.clone()
    }

    /// Waits for one persisted lifecycle event.
    ///
    /// # Errors
    ///
    /// Returns an error for database failure or timeout.
    pub async fn wait_for_run_event(
        &self,
        run_id: RunId,
        kind: RunEventKind,
        timeout: Duration,
    ) -> Result<(), AppError> {
        let deadline = Instant::now() + timeout;
        loop {
            let exists =
                control_plane_postgres::has_run_event(&self.pool, run_id.as_uuid(), kind.as_str())
                    .await
                    .map_err(component("run event query"))?;
            if exists {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(AppError::Timeout(format!(
                    "run {run_id} did not persist {}",
                    kind.as_str()
                )));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Stops admission, cancels active runs, drains supervised tasks, and
    /// closes NATS and `PostgreSQL` resources.
    ///
    /// # Errors
    ///
    /// Returns the first task or resource-shutdown failure.
    pub async fn shutdown(mut self) -> Result<(), AppError> {
        self.cancellation.cancel();
        for run in self
            .run_repository
            .recoverable_runs()
            .await
            .map_err(component("recoverable run query"))?
        {
            let command = CancelRun {
                command_id: CommandId::new(),
                run_id: run.id,
                reason: String::from("daemon shutdown"),
            };
            if let Err(error) = self.orchestrator.cancel_run(&command).await {
                tracing::warn!(run_id = %run.id, %error, "active run cancellation failed");
            }
        }

        let deadline = Instant::now() + self.shutdown_timeout;
        let mut first_error = None;
        for mut task in self.tasks.drain(..) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(error))) => {
                    first_error.get_or_insert(AppError::Task(error));
                }
                Ok(Err(error)) => {
                    first_error.get_or_insert_with(|| AppError::Task(error.to_string()));
                }
                Err(_) => {
                    task.abort();
                    drop(task.await);
                    first_error.get_or_insert_with(|| {
                        AppError::Timeout(String::from("supervised task drain timed out"))
                    });
                }
            }
        }
        if let Err(error) = self.flush_outbox(deadline).await {
            first_error.get_or_insert(error);
        }
        if let Err(error) = self.nats_client.drain().await {
            first_error.get_or_insert_with(|| AppError::Shutdown(error.to_string()));
        }
        self.pool.close().await;
        self.application_pool.close().await;
        self.service_log_pool.close().await;
        first_error.map_or(Ok(()), Err)
    }

    async fn flush_outbox(&self, deadline: Instant) -> Result<(), AppError> {
        let forge_publisher = ForgeNatsOutboxPublisher::new(self.jetstream.clone());
        let release_publisher =
            ReleaseOutboxPublisher::new(self.jetstream.clone(), self.pool.clone());
        let review_publisher = ReviewOutboxPublisher::new(
            self.jetstream.clone(),
            Arc::clone(&self.review_repository) as Arc<dyn review_service::ReviewOutboxStore>,
        );
        let event_publisher = event_adapter::EventPublisher::new(
            self.jetstream.clone(),
            Arc::new(event_postgres::PostgresProductEventOutbox::new(
                self.pool.clone(),
            )),
            self.product_event_cursor_key,
        );
        let mailbox_publisher =
            MailboxOutboxPublisher::new(self.jetstream.clone(), self.mailbox_repository.clone());
        let diagnostics = Arc::new(StdMutex::new(FlushDiagnostics::new(deadline)));
        let result = flush_until_quiescent(deadline, Arc::clone(&diagnostics), || {
            let diagnostics = Arc::clone(&diagnostics);
            let forge_publisher = forge_publisher.clone();
            let release_publisher = release_publisher.clone();
            let review_publisher = review_publisher.clone();
            let event_publisher = event_publisher.clone();
            let mailbox_publisher = mailbox_publisher.clone();
            async move {
                let forge = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Forge,
                    forge_publisher.publish_pending(self.forge.as_ref(), self.outbox_batch_size),
                    "final forge outbox flush",
                )
                .await?;
                let releases = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Release,
                    release_publisher.publish_pending(self.outbox_batch_size),
                    "final release outbox flush",
                )
                .await?;
                let reviews = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Review,
                    review_publisher.publish_pending(self.outbox_batch_size),
                    "final review outbox flush",
                )
                .await?;
                let events = flush_publisher(
                    &diagnostics,
                    FlushPublisher::ProductEvent,
                    event_publisher.publish_pending(self.outbox_batch_size),
                    "final product-event outbox flush",
                )
                .await?;
                let mailboxes = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Mailbox,
                    mailbox_publisher.publish_pending(self.outbox_batch_size),
                    "final mailbox outbox flush",
                )
                .await?;
                Ok::<_, AppError>(
                    forge == 0 && releases == 0 && reviews == 0 && events == 0 && mailboxes == 0,
                )
            }
        })
        .await;
        if result.is_err() {
            diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned")
                .log_failure(deadline);
        }
        result
    }
}
