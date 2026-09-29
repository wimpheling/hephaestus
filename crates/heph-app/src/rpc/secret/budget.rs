use crate::application::commands::InternalCommand;
use crate::rpc::{into_connect_error, mutation_receipt as load_mutation_receipt, request};

pub(super) async fn execute(
    service: &super::SecretRpc,
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
