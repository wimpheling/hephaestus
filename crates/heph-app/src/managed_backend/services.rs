//! Actual original startup/running custody, distinct from lifecycle diagnostics.
#[path = "services/startup.rs"]
mod startup;
pub use startup::StartupTicket;
pub(super) use startup::spawn;
#[path = "services/build.rs"]
mod build;
#[path = "services/closing.rs"]
mod closing;
#[path = "services/joins.rs"]
mod joins;
#[path = "services/tasks.rs"]
mod tasks;
pub use tasks::OwnedTasks;

use crate::{AppError, HephaestusApp, RunningHephaestus};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(super) enum Body {
    Empty,
    #[cfg(test)]
    TestOwner(Arc<OwnerSentinel>),
    Starting(Arc<HephaestusApp>),
    App(Arc<HephaestusApp>),
    Running {
        app: Arc<HephaestusApp>,
        running: Arc<RunningHephaestus>,
    },
}
type BuildTask = Arc<joins::JoinSlot<Result<(), Arc<AppError>>>>;
struct State {
    body: Body,
    closing: bool,
    closed: bool,
    startup: Vec<Arc<joins::JoinSlot<()>>>,
    build: Vec<BuildTask>,
    cancellation: Option<CancellationToken>,
    registry: std::sync::Weak<super::supervisor::registry::Registry>,
}
pub struct Services {
    state: Mutex<State>,
    tasks: Arc<tasks::TaskCustody>,
}
impl Default for Services {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                body: Body::Empty,
                closing: false,
                closed: false,
                startup: Vec::new(),
                build: Vec::new(),
                cancellation: None,
                registry: std::sync::Weak::new(),
            }),
            tasks: Arc::new(tasks::TaskCustody::default()),
        }
    }
}
impl Services {
    pub(super) fn begin(
        &self,
        registry: std::sync::Weak<super::supervisor::registry::Registry>,
        app: Arc<HephaestusApp>,
    ) -> Result<(), AppError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
        if !matches!(state.body, Body::Empty) {
            return Err(AppError::Readiness(
                "managed original startup already registered".into(),
            ));
        }
        state.body = if state.closed {
            Body::App(app)
        } else {
            Body::Starting(app)
        };
        state.registry = registry;
        build::initiate(&mut state, false)?;
        let closed = state.closed;
        drop(state);
        if closed {
            Err(AppError::Readiness(
                "managed startup intake is already closed; original App retained".into(),
            ))
        } else {
            Ok(())
        }
    }
    pub(super) fn attach(&self, task: JoinHandle<()>) -> Result<(), AppError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
        let duplicate = !state.startup.is_empty();
        // Retain ANY actual handle before rejecting late or duplicate registration.
        state.startup.push(joins::JoinSlot::new(task));
        let closed = state.closed;
        drop(state);
        if closed || duplicate {
            Err(AppError::Readiness(
                "managed startup closed or already registered; actual late handle retained".into(),
            ))
        } else {
            Ok(())
        }
    }
    pub fn token(&self, token: &CancellationToken) -> Result<(), AppError> {
        let registry = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
            state.cancellation = Some(token.clone());
            if state.closed {
                token.cancel();
            }
            let registry = state.registry.clone();
            drop(state);
            registry
        };
        let watched = token.clone();
        let task = tokio::spawn(async move {
            watched.cancelled().await;
            if let Some(registry) = registry.upgrade() {
                registry.close();
            }
            Ok(())
        });
        self.tasks.retain(&mut vec![task]);
        // A close can win before worker token installation.
        let closing = self
            .state
            .lock()
            .map_err(|_| AppError::Task("managed service slot poisoned".into()))?
            .registry
            .upgrade()
            .is_none_or(|registry| registry.is_closing());
        if closing {
            token.cancel();
        }
        Ok(())
    }
    pub(super) fn put(&self, body: Body) -> Result<(), AppError> {
        match self.state.lock() {
            Ok(mut state) => {
                state.body = body;
                build::initiate(&mut state, false)?;
                drop(state);
                Ok(())
            }
            Err(poisoned) => {
                poisoned.into_inner().body = body;
                Err(AppError::Task(
                    "managed service slot poisoned; original bundle retained".into(),
                ))
            }
        }
    }
    pub(super) fn facade(&self) -> Result<RunningHephaestus, AppError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
        match &state.body {
            Body::Running { running, .. } => running.managed_facade(),
            _ => Err(AppError::Readiness(
                "managed actual Running bundle is not installed".into(),
            )),
        }
    }
    pub fn healthy(&self) -> bool {
        self.state.lock().is_ok_and(|state| {
            !state.closed
                && matches!(state.body, Body::Running { .. })
                && state
                    .cancellation
                    .as_ref()
                    .is_some_and(|token| !token.is_cancelled())
                && self.tasks.healthy()
        })
    }
    pub(super) fn closed(&self) -> bool {
        self.state.lock().map_or(true, |state| state.closed)
    }
    pub fn request_close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        if let Some(token) = &state.cancellation {
            token.cancel();
        }
        if let Err(cause) = build::initiate(&mut state, false) {
            tracing::error!(%cause, "original Build cleanup registration remains held");
        }
        drop(state);
    }
    /// Starts/retries only the original Build barrier before DB/task waits.
    pub(super) fn start_cleanup(&self) -> Result<(), AppError> {
        self.request_close();
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
        let result = build::initiate(&mut state, true);
        drop(state);
        result
    }
    /// Actual joins only. Original mutation outcomes never establish resource absence.
    pub(super) async fn join_mutations(&self) -> Result<(), AppError> {
        self.request_close();
        let mut failure = None;
        loop {
            let startup = {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
                let startup = state.startup.clone();
                drop(state);
                startup
            };
            for handle in &startup {
                if let Err(cause) = handle.join().await {
                    failure.get_or_insert(cause);
                }
            }
            if let Err(cause) = self.tasks.join().await {
                failure.get_or_insert(cause);
            }
            // Startup/put can register Build after the earlier startup snapshot.
            let build = {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
                let build = state.build.clone();
                drop(state);
                build
            };
            let mut latest = Ok(());
            for handle in &build {
                match handle.join().await {
                    Ok(result) => latest = result,
                    Err(cause) => {
                        failure.get_or_insert(cause);
                    }
                }
            }
            // State→TaskRegistry matches healthy(). Transfer never takes State
            // while holding TaskRegistry. Any late actual slot forces another pass.
            let state = self
                .state
                .lock()
                .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
            let settled = state.startup.len() == startup.len()
                && state.startup.iter().all(|handle| handle.settled())
                && state.build.len() == build.len()
                && state.build.iter().all(|handle| handle.settled())
                && self.tasks.settled()?;
            drop(state);
            if settled {
                // Other actual errors remain cached in their original slots.
                if let Some(cause) = failure {
                    return Err(cause);
                }
                return latest.map_err(crate::managed_backend::custody::failure);
            }
        }
    }

    pub(super) fn working(&self) -> bool {
        self.state.lock().map_or(true, |state| {
            matches!(state.body, Body::Starting(_)) || state.closing
        })
    }
    pub(super) async fn close_owned(&self) -> Result<(), AppError> {
        closing::run(self).await
    }
}

#[cfg(test)]
#[path = "services/tests.rs"]
mod tests;

// Controlled genuine Drop resource for the same slot, never a production App.
#[cfg(test)]
pub(super) struct OwnerSentinel(pub(super) Arc<std::sync::atomic::AtomicBool>);
#[cfg(test)]
impl Drop for OwnerSentinel {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}
