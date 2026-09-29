use super::helpers::parse_id;
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use mailbox_domain::{MailboxEventId, MailboxId};
use mailbox_postgres::{MailboxOperatorAction, PostgresMailboxRepository};
use rpc_proto::messages::hephaestus::instance::v1::{
    ControlMailboxRequest, ControlMailboxResponse,
    MailboxControlAction as ProtoMailboxControlAction,
};
pub(super) async fn control_mailbox(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ControlMailboxRequest>,
) -> ServiceResult<ControlMailboxResponse> {
    let request = request.to_owned_message();
    let identity = super::helpers::mutation(
        &ctx,
        &service.authenticator,
        "ControlMailbox",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let action = match request.action.as_known() {
        Some(ProtoMailboxControlAction::Pause) => MailboxOperatorAction::Pause,
        Some(ProtoMailboxControlAction::Resume) => MailboxOperatorAction::Resume,
        Some(ProtoMailboxControlAction::Retry) => MailboxOperatorAction::Retry,
        Some(ProtoMailboxControlAction::Cancel) => MailboxOperatorAction::Cancel,
        Some(ProtoMailboxControlAction::DeadLetter) => MailboxOperatorAction::DeadLetter,
        Some(ProtoMailboxControlAction::Unspecified) | None => {
            return Err(into_connect_error(RpcError::InvalidArgument));
        }
    };
    let result = request::run_with_budget(
        &budget,
        PostgresMailboxRepository::new(service.pool.clone()).operate(
            &identity,
            action,
            parse_id::<MailboxId>(request.mailbox_id.as_option())?,
            request
                .event_id
                .as_option()
                .map(|value| parse_id::<MailboxEventId>(Some(value)))
                .transpose()?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    // The repository deliberately does not disclose whether an
    // inaccessible mailbox or event exists.
    .map_err(|_| into_connect_error(RpcError::NotFound))?;
    let receipt = super::budget::receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "agent_instance",
        "agent_instance",
    )
    .await?;
    Response::ok(ControlMailboxResponse {
        changed: result.changed,
        receipt: receipt.into(),
        ..Default::default()
    })
}
