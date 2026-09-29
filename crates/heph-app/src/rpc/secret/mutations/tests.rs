use crate::rpc::{RpcError, request};
use std::time::{Duration, Instant};

#[tokio::test]
async fn secret_operation_budget_stops_an_expired_downstream_call() {
    let budget = request::RequestBudget::from_deadline(Some(
        Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("instant supports subtraction"),
    ));
    let result = request::run_with_budget(&budget, std::future::pending::<()>()).await;
    assert_eq!(result, Err(RpcError::DeadlineExceeded));
}
