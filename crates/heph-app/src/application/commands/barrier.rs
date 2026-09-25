//! Test synchronization for update admission races.

use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Notify;
use uuid::Uuid;

/// Test-only synchronization point for the durable update admission race.
///
/// The hook is inert unless an integration test explicitly installs it. It
/// pauses after `CreateUpdate` commits and before the immediate hook attempt,
/// allowing the durable completion reconciler to win that admission race.
#[doc(hidden)]
#[cfg(feature = "test-fixtures")]
pub struct CreateUpdateAdmissionBarrier {
    committed: Notify,
    admitted: Notify,
    release: Notify,
    committed_update: Mutex<Option<Uuid>>,
    admitted_update: Mutex<Option<Uuid>>,
}

#[cfg(feature = "test-fixtures")]
impl CreateUpdateAdmissionBarrier {
    /// Creates an untriggered admission barrier.
    #[must_use]
    pub fn new() -> Self {
        Self {
            committed: Notify::new(),
            admitted: Notify::new(),
            release: Notify::new(),
            committed_update: Mutex::new(None),
            admitted_update: Mutex::new(None),
        }
    }

    /// Waits until the update transaction has committed.
    ///
    /// # Panics
    ///
    /// Panics if the test barrier mutex is poisoned.
    pub async fn wait_committed(&self) -> Uuid {
        loop {
            let notification = self.committed.notified();
            let committed_update = *self
                .committed_update
                .lock()
                .expect("committed update mutex");
            if let Some(update_id) = committed_update {
                return update_id;
            }
            notification.await;
        }
    }

    /// Waits until the durable reconciler admits the hook run.
    ///
    /// # Panics
    ///
    /// Panics if the test barrier mutex is poisoned.
    pub async fn wait_admitted(&self, update_id: Uuid) {
        loop {
            let notification = self.admitted.notified();
            let admitted_update = *self.admitted_update.lock().expect("admitted update mutex");
            if admitted_update == Some(update_id) {
                return;
            }
            notification.await;
        }
    }

    /// Releases the immediate application admission attempt.
    pub fn release(&self) {
        self.release.notify_one();
    }

    pub(super) async fn pause_before_immediate_attempt(&self, update_id: Uuid) {
        *self
            .committed_update
            .lock()
            .expect("committed update mutex") = Some(update_id);
        self.committed.notify_one();
        self.release.notified().await;
    }

    pub(crate) fn notify_reconciler_admission(&self, update_id: Uuid) {
        if *self
            .committed_update
            .lock()
            .expect("committed update mutex")
            == Some(update_id)
        {
            *self.admitted_update.lock().expect("admitted update mutex") = Some(update_id);
            self.admitted.notify_one();
        }
    }
}

#[cfg(feature = "test-fixtures")]
impl Default for CreateUpdateAdmissionBarrier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "test-fixtures")]
static CREATE_UPDATE_ADMISSION_BARRIER: OnceLock<Mutex<Option<Arc<CreateUpdateAdmissionBarrier>>>> =
    OnceLock::new();

/// Installs the integration-test admission barrier until the returned guard
/// is dropped. Only one barrier may be active in a process.
#[doc(hidden)]
#[cfg(feature = "test-fixtures")]
pub fn install_create_update_admission_barrier(
    barrier: Arc<CreateUpdateAdmissionBarrier>,
) -> CreateUpdateAdmissionBarrierGuard {
    let slot = CREATE_UPDATE_ADMISSION_BARRIER.get_or_init(|| Mutex::new(None));
    let mut current = slot.lock().expect("update admission barrier mutex");
    assert!(
        current.is_none(),
        "an update admission barrier is already active"
    );
    *current = Some(barrier);
    CreateUpdateAdmissionBarrierGuard
}

/// Removes an installed integration-test admission barrier on scope exit.
#[doc(hidden)]
#[cfg(feature = "test-fixtures")]
pub struct CreateUpdateAdmissionBarrierGuard;

#[cfg(feature = "test-fixtures")]
impl Drop for CreateUpdateAdmissionBarrierGuard {
    fn drop(&mut self) {
        if let Some(slot) = CREATE_UPDATE_ADMISSION_BARRIER.get() {
            *slot.lock().expect("update admission barrier mutex") = None;
        }
    }
}

#[cfg(feature = "test-fixtures")]
pub(super) fn installed_create_update_admission_barrier()
-> Option<Arc<CreateUpdateAdmissionBarrier>> {
    CREATE_UPDATE_ADMISSION_BARRIER
        .get()
        .and_then(|slot| slot.lock().ok()?.clone())
}

/// Signals an installed test barrier when durable reconciliation admits a
/// pending update hook.
#[doc(hidden)]
#[cfg(feature = "test-fixtures")]
pub fn notify_reconciler_update_admission(update_id: Uuid) {
    if let Some(barrier) = installed_create_update_admission_barrier() {
        barrier.notify_reconciler_admission(update_id);
    }
}
