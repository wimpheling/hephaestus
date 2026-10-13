//! Actual retained service slots; no App/native cleanup receipt.
use super::super::{Body, OwnedTasks, OwnerSentinel, Services};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
};
use tokio::sync::oneshot;

#[tokio::test]
async fn canceled_startup_join_observer_keeps_original_handle_and_cached_completion() {
    let services = Services::default();
    let (entered, arrival) = oneshot::channel();
    let (release, released) = oneshot::channel();
    services
        .attach(tokio::spawn(async move {
            entered.send(()).unwrap();
            released.await.unwrap();
        }))
        .unwrap();
    arrival.await.unwrap();
    let original = {
        services
            .state
            .lock()
            .unwrap()
            .startup
            .first()
            .unwrap()
            .clone()
    };
    {
        let mut observer = Box::pin(services.join_mutations());
        assert!(matches!(futures_util::poll!(&mut observer), Poll::Pending));
    }
    assert!(!original.settled());
    assert!(original.running());
    assert!(Arc::ptr_eq(
        &original,
        services.state.lock().unwrap().startup.first().unwrap()
    ));
    release.send(()).unwrap();
    services.join_mutations().await.unwrap();
    assert!(original.settled());
    services.join_mutations().await.unwrap(); // cached, never repoll the consumed JoinHandle
    drop(original);
    drop(services);
}
#[tokio::test]
async fn canceled_service_join_and_late_transfer_cannot_retire_unjoined_originals() {
    let services = Services::default();
    let (release, released) = oneshot::channel();
    let mut tasks = OwnedTasks::new(Some(&services));
    tasks.push(tokio::spawn(async move {
        released.await.unwrap();
        Ok(())
    }));
    drop(tasks);
    {
        let mut observer = Box::pin(services.tasks.join());
        assert!(matches!(futures_util::poll!(&mut observer), Poll::Pending));
    }
    let (late_release, late_released) = oneshot::channel();
    let mut late = OwnedTasks::new(Some(&services));
    late.push(tokio::spawn(async move {
        late_released.await.unwrap();
        Ok(())
    }));
    drop(late);
    release.send(()).unwrap();
    {
        let mut observer = Box::pin(services.tasks.join());
        assert!(matches!(futures_util::poll!(&mut observer), Poll::Pending));
    }
    late_release.send(()).unwrap();
    services.tasks.join().await.unwrap();
    services.tasks.join().await.unwrap();
    drop(services);
}
#[tokio::test]
async fn closed_before_token_install_stays_closed_and_unknown_ordinary_owner_is_retained() {
    let services = Services::default();
    services.request_close();
    let token = tokio_util::sync::CancellationToken::new();
    services.token(&token).unwrap();
    assert!(token.is_cancelled());
    assert!(services.state.lock().unwrap().closed);
    let dropped = Arc::new(AtomicBool::new(false));
    services
        .put(Body::TestOwner(Arc::new(OwnerSentinel(Arc::clone(
            &dropped,
        )))))
        .unwrap();
    assert!(services.close_owned().await.is_err());
    assert!(!dropped.load(Ordering::Acquire));
    assert!(services.state.lock().unwrap().closed);
    assert!(services.facade().is_err());
    drop(services);
    assert!(dropped.load(Ordering::Acquire));
}

#[tokio::test]
async fn late_startup_handle_is_retained_and_gated_owner_never_enters_effects() {
    let services = Services::default();
    services.request_close();
    let effects = Arc::new(AtomicBool::new(false));
    let entered = Arc::clone(&effects);
    let (admit, admitted) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        if admitted.await.is_ok() {
            entered.store(true, Ordering::Release);
        }
    });
    assert!(services.attach(task).is_err());
    drop(admit);
    services.join_mutations().await.unwrap();
    assert!(services.closed());
    assert!(!effects.load(Ordering::Acquire));
    assert!(
        services
            .state
            .lock()
            .unwrap()
            .startup
            .first()
            .unwrap()
            .settled()
    );
    drop(services);
}

#[tokio::test]
async fn canceled_close_observer_releases_activity_but_never_closed_admission() {
    let services = Services::default();
    let (release, released) = oneshot::channel();
    services
        .attach(tokio::spawn(async move {
            released.await.unwrap();
        }))
        .unwrap();
    {
        let mut observer = Box::pin(services.close_owned());
        assert!(matches!(futures_util::poll!(&mut observer), Poll::Pending));
        assert!(services.state.lock().unwrap().closing);
    }
    assert!(!services.state.lock().unwrap().closing);
    assert!(services.closed());
    {
        let mut retry = Box::pin(services.close_owned());
        assert!(matches!(futures_util::poll!(&mut retry), Poll::Pending));
        assert!(services.state.lock().unwrap().closing);
    }
    release.send(()).unwrap();
    let failure = services.close_owned().await.unwrap_err();
    assert!(failure.to_string().contains("ordinary Build/OCI/Gateway"));
    assert!(!services.state.lock().unwrap().closing);
    assert!(services.closed());
    drop(services);
}
#[tokio::test]
async fn startup_registered_real_build_slot_is_joined_before_final_predicate() {
    use super::super::joins::JoinSlot;
    let services = Arc::new(Services::default());
    let dropped = Arc::new(AtomicBool::new(false));
    let owner = Arc::new(OwnerSentinel(Arc::clone(&dropped)));
    let observed_owner = Arc::downgrade(&owner);
    let (startup_release, startup_released) = oneshot::channel();
    let (build_release, build_released) = oneshot::channel();
    let (registered, registration) = oneshot::channel();
    let original = Arc::clone(&services);
    services
        .attach(tokio::spawn(async move {
            startup_released.await.unwrap();
            // Controlled real handle/owner registration, not App/Build/native proof.
            let task = tokio::spawn(async move {
                let retained = owner;
                build_released.await.unwrap();
                drop(retained);
                Ok::<(), Arc<crate::AppError>>(())
            });
            original
                .state
                .lock()
                .unwrap()
                .build
                .push(JoinSlot::new(task));
            registered.send(()).unwrap();
        }))
        .unwrap();
    let mut observer = Box::pin(services.join_mutations());
    assert!(matches!(futures_util::poll!(&mut observer), Poll::Pending));
    startup_release.send(()).unwrap();
    registration.await.unwrap();
    // SAME in-flight observation began before Build registration; old snapshot misses it.
    assert!(matches!(futures_util::poll!(&mut observer), Poll::Pending));
    drop(observer);
    assert!(observed_owner.upgrade().is_some());
    assert!(!dropped.load(Ordering::Acquire));
    build_release.send(()).unwrap();
    services.join_mutations().await.unwrap();
    assert!(
        services
            .state
            .lock()
            .unwrap()
            .build
            .iter()
            .all(|handle| handle.settled())
    );
    assert!(dropped.load(Ordering::Acquire));
    services.join_mutations().await.unwrap();
    drop(services);
}

#[tokio::test]
async fn panicked_close_observer_resets_activity_and_retains_original_startup_slot() {
    let services = Arc::new(Services::default());
    let (release, released) = oneshot::channel();
    services
        .attach(tokio::spawn(async move {
            released.await.unwrap();
        }))
        .unwrap();
    let original = Arc::clone(&services);
    let observer = tokio::spawn(async move {
        let mut close = Box::pin(original.close_owned());
        assert!(matches!(futures_util::poll!(&mut close), Poll::Pending));
        panic!("controlled closing observer unwind");
    });
    assert!(observer.await.unwrap_err().is_panic());
    assert!(!services.state.lock().unwrap().closing);
    assert!(services.closed());
    assert!(!services.state.lock().unwrap().startup[0].settled());
    release.send(()).unwrap();
    assert!(services.close_owned().await.is_err());
    assert!(services.state.lock().unwrap().startup[0].settled());
    assert!(!services.state.lock().unwrap().closing);
    drop(services);
}

#[tokio::test]
async fn rejected_late_startup_handle_still_forces_the_join_observation_to_wait() {
    let services = Services::default();
    let (first_release, first_released) = oneshot::channel();
    let (first_done, first_completed) = oneshot::channel();
    services
        .attach(tokio::spawn(async move {
            first_released.await.unwrap();
            first_done.send(()).unwrap();
        }))
        .unwrap();
    let mut observer = Box::pin(services.join_mutations());
    assert!(matches!(futures_util::poll!(&mut observer), Poll::Pending));
    let (admit, admitted) = oneshot::channel::<()>();
    let (late_release, late_released) = oneshot::channel();
    let late = tokio::spawn(async move {
        assert!(admitted.await.is_err());
        late_released.await.unwrap(); // controlled held owner, no startup effects
    });
    assert!(services.attach(late).is_err());
    drop(admit);
    first_release.send(()).unwrap();
    first_completed.await.unwrap();
    assert!(matches!(futures_util::poll!(&mut observer), Poll::Pending));
    assert_eq!(services.state.lock().unwrap().startup.len(), 2);
    late_release.send(()).unwrap();
    observer.await.unwrap();
    assert!(
        services
            .state
            .lock()
            .unwrap()
            .startup
            .iter()
            .all(|handle| handle.settled())
    );
    drop(services);
}
