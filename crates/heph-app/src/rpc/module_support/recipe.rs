use super::MutationReceipts;
use crate::rpc::RpcError;
use recipe_application::{CommandIdentity, CommandReceipt, DeploymentId};
use rpc_proto::messages::hephaestus::common::v1::{Cursor, MutationReceipt, OpaqueId};
use uuid::Uuid;

impl MutationReceipts {
    pub(in crate::rpc) fn recipe_receipt(
        &self,
        receipt: &CommandReceipt,
        command: CommandIdentity,
        project: Uuid,
        deployment: DeploymentId,
    ) -> Result<MutationReceipt, RpcError> {
        let event = &receipt.event;
        if receipt.command != command
            || receipt.deployment_id != deployment
            || event.event_id.is_nil()
            || event.scope_kind != "project"
            || event.scope_id != project
            || event.cursor <= 0
            || event.aggregate_version <= 0
            || receipt.deployment_version == 0
        {
            return Err(RpcError::FailedPrecondition);
        }
        Ok(MutationReceipt {
            event_id: OpaqueId {
                value: event.event_id.to_string(),
                ..Default::default()
            }
            .into(),
            committed_cursor: Cursor {
                value: self
                    .cursor_codec
                    .encode(&event.scope_kind, event.scope_id, event.cursor),
                ..Default::default()
            }
            .into(),
            aggregate_version: u64::try_from(event.aggregate_version)
                .map_err(|_| RpcError::FailedPrecondition)?,
            ..Default::default()
        })
    }
}
