use crate::rpc::{RpcError, into_connect_error, request::RequestBudget};
use connectrpc::ConnectError;
use std::{future::Future, time::Duration};
use tokio::sync::mpsc;

pub(super) async fn run_with_stream_budget<OperationOutput, Item, Operation>(
    budget: &RequestBudget,
    sender: &mpsc::Sender<Result<Item, ConnectError>>,
    operation: Operation,
) -> Result<Option<OperationOutput>, RpcError>
where
    Operation: Future<Output = OperationOutput>,
{
    tokio::select! {
        result = crate::rpc::request::run_with_budget(budget, operation) => result.map(Some),
        () = sender.closed() => {
            budget.cancel();
            Ok(None)
        }
    }
}

pub(super) async fn send_with_stream_budget<T>(
    budget: &RequestBudget,
    sender: &mpsc::Sender<Result<T, ConnectError>>,
    item: Result<T, ConnectError>,
) -> bool {
    matches!(
        run_with_stream_budget(budget, sender, sender.send(item)).await,
        Ok(Some(Ok(())))
    )
}

pub(super) async fn send_stream_error<T>(
    budget: &RequestBudget,
    sender: &mpsc::Sender<Result<T, ConnectError>>,
    error: RpcError,
) {
    if matches!(error, RpcError::DeadlineExceeded) {
        let _ = sender.try_send(Err(into_connect_error(error)));
        budget.cancel();
    } else {
        let _ = send_with_stream_budget(budget, sender, Err(into_connect_error(error))).await;
    }
}

pub(super) async fn sleep_with_stream_budget<T>(
    budget: &RequestBudget,
    sender: &mpsc::Sender<Result<T, ConnectError>>,
    duration: Duration,
) -> Result<Option<()>, RpcError> {
    run_with_stream_budget(budget, sender, tokio::time::sleep(duration)).await
}

#[cfg(test)]
mod tests {
    use super::{run_with_stream_budget, send_stream_error};
    use crate::rpc::{RpcError, request::RequestBudget};
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn receiver_drop_cancels_pending_operation() {
        let budget = RequestBudget::unbounded();
        let cancellation = budget.cancellation_token();
        let (sender, receiver) = mpsc::channel::<Result<(), connectrpc::ConnectError>>(1);
        drop(receiver);

        let result = run_with_stream_budget(&budget, &sender, std::future::pending::<()>()).await;

        assert_eq!(result, Ok(None));
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn expired_budget_queues_best_effort_terminal_error() {
        let budget = RequestBudget::from_deadline(Some(
            Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("instant supports subtraction"),
        ));
        let cancellation = budget.cancellation_token();
        let (sender, mut receiver) = mpsc::channel::<Result<(), connectrpc::ConnectError>>(1);

        let result = run_with_stream_budget(&budget, &sender, std::future::pending::<()>()).await;
        assert_eq!(result, Err(RpcError::DeadlineExceeded));
        send_stream_error::<()>(&budget, &sender, RpcError::DeadlineExceeded).await;

        let response = receiver.recv().await.expect("deadline response");
        let Err(error) = response else {
            panic!("deadline must be returned as a stream error");
        };
        assert_eq!(error.code, connectrpc::ErrorCode::DeadlineExceeded);
        assert!(cancellation.is_cancelled());
    }
}
