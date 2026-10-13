//! Original task handle and actual cached join, never a termination inference.
use super::node::Node;
use std::sync::Arc;
use tokio::task::{JoinError, JoinHandle};
struct Slot<T> {
    handle: JoinHandle<T>,
    result: Option<Result<T, Arc<JoinError>>>,
}
pub(super) struct Task<T>(tokio::sync::Mutex<Slot<T>>);
impl<T: Clone> Task<T> {
    pub(super) fn new(handle: JoinHandle<T>) -> Arc<Self> {
        Arc::new(Self(tokio::sync::Mutex::new(Slot {
            handle,
            result: None,
        })))
    }
    pub(super) fn joined(&self) -> bool {
        self.0.try_lock().is_ok_and(|slot| slot.result.is_some())
    }
    pub(super) fn ready(&self) -> bool {
        // Only a polling hint. Settlement requires the cached actual join.
        self.0
            .try_lock()
            .is_ok_and(|slot| slot.handle.is_finished())
    }
    pub(super) async fn join(&self, owner: &Node) -> Result<T, Arc<JoinError>> {
        let mut slot = self.0.lock().await;
        if let Some(result) = &slot.result {
            return result.clone();
        }
        // The original handle stays in this retained slot on cancellation.
        let result = (&mut slot.handle).await.map_err(Arc::new);
        if let Err(error) = &result {
            owner.error(error.clone());
        }
        slot.result = Some(result.clone());
        result
    }
}

#[cfg(test)]
pub(super) struct WatcherControl {
    pub(super) entered: tokio::sync::Notify,
    pub(super) released: tokio::sync::Notify,
    pub(super) panic: bool,
}
#[cfg(test)]
impl WatcherControl {
    pub(super) fn new(panic: bool) -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            released: tokio::sync::Notify::new(),
            panic,
        })
    }
}
