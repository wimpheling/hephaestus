//! Exercises the actual registered Ticket, not an unowned arbitrary future.
use super::*;
use crate::rpc::{RequestBudget, run_with_budget};
#[tokio::test]
async fn canceled_rpc_waiter_keeps_actual_job_and_original_owner() {
    let (service, mut control) = service(false);
    let ticket = service
        .submit(Request::install(identity(), plan()).unwrap())
        .unwrap();
    control.entered.recv().await.unwrap();
    let budget = RequestBudget::unbounded();
    budget.cancel();
    assert!(matches!(
        run_with_budget(&budget, ticket.wait()).await,
        Err(crate::rpc::RpcError::Canceled)
    ));
    assert!(!control.owner_dropped.load(Ordering::SeqCst));
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    control.release.add_permits(1);
    assert!(service.join_jobs().await.is_err());
    assert!(!control.owner_dropped.load(Ordering::SeqCst));
    drop(service);
    assert!(control.owner_dropped.load(Ordering::SeqCst));
}
