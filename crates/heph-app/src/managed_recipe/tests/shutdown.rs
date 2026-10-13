//! Real recipe job registry/join protocol with controlled engine outcomes only.
use super::{Actions, Arc, ManagedRecipe, Outcome, Request, identity, plan};
use std::{
    sync::{Mutex, Weak},
    task::Poll,
};
use tokio::sync::{Semaphore, mpsc};

fn denial() -> Outcome {
    Err(Arc::new(super::Failure::Execution(
        crate::recipe_engine::RecipeApplicationError::Deployment(
            recipe_application::DeploymentError::AuthorizationDenied,
        ),
    )))
}
struct HeldDenial {
    entered: mpsc::UnboundedSender<()>,
    release: Arc<Semaphore>,
}
#[async_trait::async_trait]
impl Actions for HeldDenial {
    async fn run(&self, _: &Request) -> Outcome {
        self.entered.send(()).unwrap();
        self.release.acquire().await.unwrap().forget();
        denial()
    }
}
#[tokio::test]
async fn known_logical_denial_remains_recorded_without_failing_actual_join_barrier() {
    let (service, mut control) = super::controlled_service(None, Some(0));
    let ticket = service
        .submit(Request::install(identity(), plan()).unwrap())
        .unwrap();
    control.entered.recv().await.unwrap();
    assert!(ticket.wait().await.is_err());
    service.join_jobs().await.unwrap();
    {
        let state = service.state();
        let entry = &state.entries[&1];
        assert!(entry.joined);
        assert!(entry.outcome.as_ref().unwrap().is_err());
        assert!(entry.join_error.is_none());
        drop(state);
    }
    // Join success is not a ledger/physical qualification or delivery ACK.
    service.join_jobs().await.unwrap();
    drop(service);
}
#[tokio::test]
async fn observer_cancel_preserves_both_fifo_mutations_before_a_final_observation() {
    let (entered, mut observed) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let service = Arc::new(ManagedRecipe {
        registry: Weak::new(),
        actions: Arc::new(HeldDenial {
            entered,
            release: Arc::clone(&release),
        }),
        jobs: Mutex::default(),
    });
    let first = service
        .submit(Request::install(identity(), plan()).unwrap())
        .unwrap();
    observed.recv().await.unwrap();
    let second = service
        .submit(Request::install(identity(), plan()).unwrap())
        .unwrap();
    {
        let mut join_waiter = Box::pin(service.join_jobs());
        assert!(matches!(
            futures_util::poll!(&mut join_waiter),
            Poll::Pending
        ));
    }
    assert!(
        service
            .submit(Request::install(identity(), plan()).unwrap())
            .is_err()
    );
    release.add_permits(1);
    assert!(first.wait().await.is_err());
    observed.recv().await.unwrap();
    {
        let mut join_waiter = Box::pin(service.join_jobs());
        assert!(matches!(
            futures_util::poll!(&mut join_waiter),
            Poll::Pending
        ));
    }
    release.add_permits(1);
    assert!(second.wait().await.is_err());
    service.join_jobs().await.unwrap();
    {
        let state = service.state();
        assert!(
            state
                .entries
                .values()
                .all(|entry| entry.joined && entry.outcome.is_some())
        );
        drop(state);
    }
    // Only HERE could the production caller begin its separate actual native check.
    drop(service);
}
