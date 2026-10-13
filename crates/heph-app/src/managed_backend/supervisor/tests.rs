//! Controlled ownership scheduling and genuine local descriptor/flock primitives.
//! These checks are not native VM stop, DB reconciliation or readiness evidence.
use super::{
    Control, ManagedBackendSupervisor, ManagedHeldReason, ManagedSupervisorPhase,
    ManagedSupervisorStatus, NodeLease, PreparationTicket,
    registry::{Action, Registry},
};
use crate::AppError;
use std::{error::Error, fmt, sync::Arc, time::Duration};
use tokio::sync::{oneshot, watch};
use volume_local::{ProspectiveVolumeRootBootstrap, VolumeRootOwner};
use volume_trait::VolumeRootNamespaceId;

fn lease() -> (NodeLease, Arc<Control>) {
    let control = Arc::new(Control::default());
    let (_, status) = watch::channel(ManagedSupervisorStatus::default());
    (
        NodeLease {
            control: Arc::clone(&control),
            status,
        },
        control,
    )
}

#[test]
fn abandoned_lease_seals_intake_and_never_grants_readiness() {
    let (lease, control) = lease();
    assert!(lease.require_ready().is_err());
    assert!(!control.is_closing());
    drop(lease);
    assert!(control.is_closing());
    control.close();
    assert!(control.is_closing());
}

#[tokio::test]
async fn delivered_lease_transfers_without_closing() {
    let (lease, control) = lease();
    let (sender, result) = oneshot::channel();
    sender.send(Ok(())).unwrap();
    let lease = PreparationTicket {
        lease: Some(lease),
        result,
    }
    .wait()
    .await
    .unwrap();
    assert!(!control.is_closing());
    assert!(lease.require_ready().is_err());
    drop(lease);
    assert!(control.is_closing());
}

#[tokio::test]
async fn cancelled_result_waiter_closes_without_aborting_sender() {
    let (lease, control) = lease();
    let (sender, result) = oneshot::channel();
    let ticket = PreparationTicket {
        lease: Some(lease),
        result,
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(1), ticket.wait())
            .await
            .is_err()
    );
    assert!(control.is_closing());
    assert!(sender.send(Ok(())).is_err());
}

#[tokio::test]
async fn failed_result_delivery_preserves_lease_close() {
    let (lease, control) = lease();
    let (sender, result) = oneshot::channel();
    drop(sender);
    assert!(
        PreparationTicket {
            lease: Some(lease),
            result
        }
        .wait()
        .await
        .is_err()
    );
    assert!(control.is_closing());
}

#[tokio::test]
async fn empty_service_joins_only_after_stopped() {
    let mut service = ManagedBackendSupervisor::start();
    assert_eq!(service.status().phase, ManagedSupervisorPhase::Empty);
    service.shutdown(Duration::from_secs(1)).await.unwrap();
    assert_eq!(service.status().phase, ManagedSupervisorPhase::Stopped);
    assert!(service.task.is_none());
    service.shutdown(Duration::from_secs(1)).await.unwrap();
    drop(service);
}

#[tokio::test]
async fn last_service_owner_drop_closes_without_sender_cycle() {
    let service = ManagedBackendSupervisor::start();
    let control = Arc::clone(&service.registry.control);
    assert!(!service.task.as_ref().unwrap().is_finished());
    let registry = Arc::downgrade(&service.registry);
    drop(service);
    assert!(control.is_closing());
    tokio::time::timeout(Duration::from_secs(1), async {
        while registry.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[test]
fn uncertain_acquisition_never_stops_or_retries_and_excludes_inflight() {
    let (registry, status) = Registry::new();
    registry.failed_preparation(
        AppError::Task("controlled acquisition uncertainty".into()),
        true,
    );
    registry.control.close();
    assert!(matches!(registry.action(false, true), Action::Wait));
    assert!(matches!(registry.action(true, true), Action::Wait));
    assert!(registry.retry().is_err());
    assert_eq!(
        status.borrow().held_reason,
        Some(ManagedHeldReason::Acquisition)
    );
    assert_eq!(status.borrow().phase, ManagedSupervisorPhase::Held);
}

struct HeldRoot {
    _original: ProspectiveVolumeRootBootstrap,
}
impl fmt::Debug for HeldRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HeldRoot(original descriptor)")
    }
}
impl fmt::Display for HeldRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("controlled failure retains original volume preparation")
    }
}
impl Error for HeldRoot {}

#[test]
fn retained_original_error_keeps_actual_prospective_root_flock_after_waiter_drop() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    let namespace = VolumeRootNamespaceId::from_uuid(uuid::Uuid::from_u128(91)).unwrap();
    let original =
        VolumeRootOwner::prepare_empty_with_namespace(&path, "supervisor-test", namespace).unwrap();
    let (registry, status) = Registry::new();
    let delivered = registry.failed_preparation(
        HeldRoot {
            _original: original,
        },
        true,
    );
    let caller_error = crate::managed_backend::custody::failure(delivered);
    drop(caller_error);
    assert!(
        VolumeRootOwner::prepare_empty_with_namespace(&path, "supervisor-test", namespace).is_err()
    );
    assert!(std::fs::read_dir(&path).unwrap().next().is_none());
    registry.control.close();
    assert!(matches!(registry.action(true, true), Action::Wait));
    assert_eq!(status.borrow().phase, ManagedSupervisorPhase::Held);
    let retained = registry.failure();
    assert!(retained.source().is_some());
    drop(retained);
    drop(registry);
    // Only explicit loss of this controlled registry ends the original flock.
    // Production keeps its service/runtime while Held; this is not a stop proof.
    let prepared =
        VolumeRootOwner::prepare_empty_with_namespace(&path, "supervisor-test", namespace).unwrap();
    drop(prepared);
}

#[tokio::test]
async fn stalled_constructor_job_does_not_block_cleanup_schedule() {
    // Real local descriptor/flock custody inside a controlled stalled job. The
    // schedule below is the production claim mechanism AFTER node qualification;
    // its signal is a scheduling observation, never a physical/native stop proof.
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    let namespace = VolumeRootNamespaceId::from_uuid(uuid::Uuid::from_u128(92)).unwrap();
    let original =
        VolumeRootOwner::prepare_empty_with_namespace(&path, "stalled-job", namespace).unwrap();
    let (entered, observing) = oneshot::channel();
    let (resume, stalled) = oneshot::channel();
    let mut preparation = tokio::task::JoinSet::new();
    preparation.spawn(async move {
        entered.send(()).unwrap();
        stalled.await.unwrap();
        drop(original);
    });
    tokio::time::timeout(Duration::from_secs(1), observing)
        .await
        .unwrap()
        .unwrap();
    assert!(!preparation.is_empty());
    let mut cleanup = tokio::task::JoinSet::new();
    let mut schedule = super::registry::CleanupSchedule::default();
    assert_eq!(schedule.claim(cleanup.is_empty()), Some(3));
    let (closed, observing) = oneshot::channel();
    cleanup.spawn(async move {
        closed.send(()).unwrap();
    });
    tokio::time::timeout(Duration::from_secs(1), observing)
        .await
        .unwrap()
        .unwrap();
    assert!(!preparation.is_empty());
    assert!(
        VolumeRootOwner::prepare_empty_with_namespace(&path, "stalled-job", namespace).is_err()
    );
    assert_eq!(schedule.claim(cleanup.is_empty()), None);
    assert!(!schedule.request_retry());
    tokio::time::timeout(Duration::from_secs(1), cleanup.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    schedule.finish();
    // Retry can finish while constructor custody is still outstanding.
    assert!(schedule.request_retry());
    assert!(!schedule.request_retry());
    assert_eq!(schedule.claim(cleanup.is_empty()), Some(1));
    assert_eq!(schedule.claim(true), None);
    schedule.finish();
    assert_eq!(schedule.claim(true), None);
    resume.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), preparation.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(VolumeRootOwner::prepare_empty_with_namespace(&path, "stalled-job", namespace).is_ok());
}

#[test]
fn no_node_never_closes_and_stopped_requires_all_jobs_settled() {
    let (registry, _) = Registry::new();
    registry.control.close();
    assert!(matches!(registry.action(false, true), Action::Wait));
    registry.prepared();
    assert!(matches!(registry.action(false, true), Action::Wait));
    assert!(matches!(registry.action(false, false), Action::Wait));
    assert!(matches!(registry.action(true, true), Action::Stop));
}

#[test]
fn late_constructor_completion_does_not_reopen_closing_diagnostics() {
    let (registry, status) = Registry::new();
    registry.control.close();
    registry.prepared();
    assert_eq!(status.borrow().phase, ManagedSupervisorPhase::Closing);
    registry.record(
        AppError::Readiness("controlled incomplete reconciliation".into()),
        ManagedHeldReason::ReconciliationUnavailable,
    );
    registry.prepared();
    assert_eq!(status.borrow().phase, ManagedSupervisorPhase::Held);
    assert_eq!(
        status.borrow().held_reason,
        Some(ManagedHeldReason::ReconciliationUnavailable)
    );
    assert!(registry.control.is_closing());
}
