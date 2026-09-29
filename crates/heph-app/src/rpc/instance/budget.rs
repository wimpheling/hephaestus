use crate::application::commands::InternalCommand;
use crate::rpc::{into_connect_error, mutation_receipt as load_mutation_receipt, request};

pub(super) async fn execute(
    service: &super::InstanceRpc,
    budget: &request::RequestBudget,
    identity: &identity_domain::AuthenticatedIdentity,
    command: InternalCommand,
) -> Result<serde_json::Value, connectrpc::ConnectError> {
    request::run_with_budget(budget, service.execute(identity, command))
        .await
        .map_err(into_connect_error)?
        .map_err(into_connect_error)
}

pub(super) async fn receipt(
    budget: &request::RequestBudget,
    receipts: &super::MutationReceipts,
    occurrence_id: identity_domain::RequestId,
    actor_id: identity_domain::UserId,
    aggregate_type: &str,
    primary_scope_kind: &str,
) -> Result<rpc_proto::messages::hephaestus::common::v1::MutationReceipt, connectrpc::ConnectError>
{
    request::run_with_budget(
        budget,
        load_mutation_receipt(
            receipts,
            occurrence_id,
            actor_id,
            aggregate_type,
            primary_scope_kind,
        ),
    )
    .await
    .map_err(into_connect_error)?
}

#[cfg(test)]
mod tests {
    use crate::rpc::request;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn downstream_operation_stops_at_transport_deadline() {
        let deadline = Instant::now() + Duration::from_millis(10);
        let budget = request::RequestBudget::from_deadline(Some(deadline));
        let result = request::run_with_budget(&budget, std::future::pending::<()>()).await;
        assert!(matches!(
            result,
            Err(crate::rpc::RpcError::DeadlineExceeded)
        ));
    }

    #[tokio::test]
    async fn downstream_operation_stops_when_request_is_canceled() {
        let budget = request::RequestBudget::from_deadline(None);
        let canceled = budget.cancellation_token();
        canceled.cancel();
        let result = request::run_with_budget(&budget, std::future::pending::<()>()).await;
        assert!(matches!(result, Err(crate::rpc::RpcError::Canceled)));
    }
}
