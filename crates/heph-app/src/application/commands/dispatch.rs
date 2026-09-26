//! Routes typed commands to their domain-specific executors.

use super::{release, secret, state::InternalCommandState, types::InternalCommand};
use identity_domain::AuthenticatedIdentity;
use serde_json::Value;
use std::error::Error;

/// Executes one idempotent internal command.
pub async fn dispatch(
    state: &InternalCommandState,
    identity: &AuthenticatedIdentity,
    command: InternalCommand,
) -> Result<Value, Box<dyn Error>> {
    if matches!(
        &command,
        InternalCommand::ImportAgent { .. }
            | InternalCommand::CreateAttachment { .. }
            | InternalCommand::SetAttachmentEnabled { .. }
            | InternalCommand::RemoveAttachment { .. }
            | InternalCommand::ReviseInstance { .. }
            | InternalCommand::ReviseCapabilities { .. }
            | InternalCommand::CreateUpdate { .. }
            | InternalCommand::RecoverUpdate { .. }
    ) {
        release::dispatch_release(state, identity, command).await
    } else {
        secret::dispatch_secret(state, identity, command).await
    }
}
