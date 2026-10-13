//! Actual original startup/running custody, distinct from lifecycle diagnostics.
#[path = "services/startup.rs"]
mod startup;
pub use startup::StartupTicket;
pub(super) use startup::spawn;
#[path = "services/closing.rs"]
mod closing;
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
struct State {
    body: Body,
    closing: bool,
    startup: Option<JoinHandle<()>>,
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
                startup: None,
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
        state.body = Body::Starting(app);
        state.registry = registry;
        drop(state);
        Ok(())
    }
    pub(super) fn attach(&self, task: JoinHandle<()>) -> Result<(), AppError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
        if state.startup.is_some() {
            return Err(AppError::Task(
                "managed startup already owns an original task".into(),
            ));
        }
        state.startup = Some(task);
        drop(state);
        Ok(())
    }
    pub fn token(&self, token: &CancellationToken) -> Result<(), AppError> {
        let registry = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
            state.cancellation = Some(token.clone());
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
            matches!(state.body, Body::Running { .. })
                && state
                    .cancellation
                    .as_ref()
                    .is_some_and(|token| !token.is_cancelled())
                && self.tasks.healthy()
        })
    }
    pub fn request_close(&self) {
        if let Ok(state) = self.state.lock() {
            if let Some(token) = &state.cancellation {
                token.cancel();
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
