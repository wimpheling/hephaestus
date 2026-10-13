//! Borrow actual task handles in retained slots; cache every terminal join.
use crate::AppError;
use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::Mutex,
    task::{JoinError, JoinHandle},
};

struct State<T> {
    task: Option<JoinHandle<T>>,
    result: Option<Result<T, Arc<JoinError>>>,
}
pub(super) struct JoinSlot<T> {
    state: Mutex<State<T>>,
    settled: AtomicBool,
}
impl<T: Clone> JoinSlot<T> {
    pub(super) fn new(task: JoinHandle<T>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                task: Some(task),
                result: None,
            }),
            settled: AtomicBool::new(false),
        })
    }
    pub(super) fn settled(&self) -> bool {
        self.settled.load(Ordering::Acquire)
    }
    pub(super) fn running(&self) -> bool {
        self.state
            .try_lock()
            .is_ok_and(|state| state.task.as_ref().is_some_and(|task| !task.is_finished()))
    }
    pub(super) fn snapshot(&self) -> Option<Result<T, Arc<JoinError>>> {
        self.state
            .try_lock()
            .ok()
            .and_then(|state| state.result.clone())
    }
    pub(super) async fn join(&self) -> Result<T, AppError> {
        let mut state = self.state.lock().await;
        if state.result.is_none() {
            let task = state.task.as_mut().ok_or_else(|| {
                AppError::Task("original task has neither handle nor terminal join".into())
            })?;
            let result = task.await.map_err(Arc::new);
            // No await between observing completion, caching it and retiring the consumed future.
            state.result = Some(result);
            drop(state.task.take());
            self.settled.store(true, Ordering::Release);
        }
        let result = state
            .result
            .clone()
            .ok_or_else(|| AppError::Task("original terminal join missing".into()))?;
        drop(state);
        result.map_err(|cause| crate::managed_backend::custody::failure(OriginalJoinError(cause)))
    }
}
#[derive(Debug)]
struct OriginalJoinError(Arc<JoinError>);
impl fmt::Display for OriginalJoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0.as_ref(), f)
    }
}
impl Error for OriginalJoinError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}
