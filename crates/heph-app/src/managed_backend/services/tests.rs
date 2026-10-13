//! Controlled original service custody; not native cleanup or readiness receipts.
use super::{OwnedTasks, Services};
use crate::managed_backend::supervisor::registry::Registry;
use tokio::sync::oneshot;

// Ticket and waiter are consumed by their actual awaited owners; a later drop is invalid.
#[allow(clippy::significant_drop_tightening)]
#[tokio::test]
async fn dropped_startup_waiter_seals_actual_registry_without_dropping_task() {
    let registry = Registry::test_empty();
    let (sender, receiver) = oneshot::channel();
    let ticket =
        super::startup::StartupTicket::test_ticket(receiver, std::sync::Arc::downgrade(&registry));
    let (entered, observed) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let task = tokio::spawn(async move {
        entered.send(()).unwrap();
        released.await.unwrap();
        drop(sender);
    });
    let waiter = tokio::spawn(ticket.wait());
    observed.await.unwrap();
    waiter.abort();
    assert!(waiter.await.err().unwrap().is_cancelled());
    assert!(registry.is_closing());
    assert!(!task.is_finished());
    release.send(()).unwrap();
    task.await.unwrap();
    drop(registry);
}

#[tokio::test]
async fn dropped_spawn_collection_retains_actual_unfinished_handle_until_join() {
    let services = Services::default();
    let (entered, observed) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let mut tasks = OwnedTasks::new(Some(&services));
    tasks.push(tokio::spawn(async move {
        entered.send(()).unwrap();
        released.await.unwrap();
        Ok(())
    }));
    observed.await.unwrap();
    drop(tasks);
    assert!(services.tasks.healthy());
    release.send(()).unwrap();
    services.tasks.join().await.unwrap();
    assert!(!services.tasks.healthy());
    drop(services);
}

#[test]
fn late_signal_cannot_create_ready_without_original_bundle() {
    let registry = Registry::test_empty();
    assert!(registry.require_recovered().is_err());
    assert!(registry.listeners_ready().is_err());
    registry.close();
    assert!(registry.listeners_ready().is_err());
    assert!(registry.require_ready().is_err());
    assert!(registry.is_closing());
    drop(registry);
}

#[tokio::test]
async fn startup_panic_keeps_actual_original_owner_in_service_slot() {
    use super::{Body, OwnerSentinel};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let services = Arc::new(Services::default());
    let dropped = Arc::new(AtomicBool::new(false));
    let original = Arc::new(OwnerSentinel(Arc::clone(&dropped)));
    let observer = Arc::downgrade(&original);
    services
        .put(Body::TestOwner(Arc::clone(&original)))
        .unwrap();
    drop(original);
    let retained = Arc::clone(&services);
    let (entered, arrival) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let task = tokio::spawn(async move {
        let owner = {
            let state = retained.state.lock().unwrap();
            let Body::TestOwner(owner) = &state.body else {
                panic!("actual owner missing");
            };
            let owner = Arc::clone(owner);
            drop(state);
            owner
        };
        entered.send(()).unwrap();
        released.await.unwrap();
        assert!(!owner.0.load(Ordering::Acquire));
        panic!("controlled startup panic");
    });
    services.attach(task).unwrap();
    arrival.await.unwrap();
    release.send(()).unwrap();
    let failure = services.close_owned().await.unwrap_err();
    let crate::AppError::ManagedBackend { source } = failure else {
        panic!("actual JoinError must be retained");
    };
    assert!(
        source
            .downcast_ref::<tokio::task::JoinError>()
            .is_some_and(tokio::task::JoinError::is_panic)
    );
    assert!(observer.upgrade().is_some());
    assert!(!dropped.load(Ordering::Acquire));
    assert!(!services.healthy());
    assert!(services.facade().is_err());
    assert!(services.close_owned().await.is_err());
    assert!(!dropped.load(Ordering::Acquire));
    drop(services);
    assert!(observer.upgrade().is_none());
    assert!(dropped.load(Ordering::Acquire));
}
