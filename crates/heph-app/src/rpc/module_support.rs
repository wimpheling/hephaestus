//! Shared RPC receipt and transport primitives.

use super::{RpcError, into_connect_error};
use event_application::MutationReceiptReader;
use sha2::{Digest, Sha256};
use std::sync::Arc;

const MEDIATOR_KEY_DOMAIN: &[u8] = b"hephaestus-rpc-mediator-v1\0";
/// Maximum request body accepted by the Connect transport.
pub const GLOBAL_MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Maximum response message accepted by the Connect transport.
pub const GLOBAL_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum request deadline accepted by the Connect transport.
pub const MAX_DEADLINE_SECONDS: u64 = 60;
/// Default request deadline used when a client does not provide one.
pub const DEFAULT_DEADLINE_SECONDS: u64 = 30;
/// Idle timeout between messages on a streaming request.
pub const STREAM_IDLE_SECONDS: u64 = 10;

#[derive(Clone)]
/// Loads committed mutation receipts for RPC responses.
pub struct MutationReceipts {
    application: Arc<dyn MutationReceiptReader>,
    cursor_codec: crate::event_cursor::EventCursorCodec,
}

impl MutationReceipts {
    /// Creates a receipt loader using the given application reader and cursor key.
    pub fn new(application: Arc<dyn MutationReceiptReader>, cursor_key: [u8; 32]) -> Self {
        Self {
            application,
            cursor_codec: crate::event_cursor::EventCursorCodec::new(cursor_key),
        }
    }

    async fn load(
        &self,
        occurrence_id: identity_domain::RequestId,
        actor_id: identity_domain::UserId,
        aggregate_type: &str,
        primary_scope_kind: &str,
    ) -> Result<rpc_proto::messages::hephaestus::common::v1::MutationReceipt, ConnectReceiptError>
    {
        let row = self
            .application
            .load(occurrence_id, actor_id, aggregate_type, primary_scope_kind)
            .await
            .map_err(ConnectReceiptError::Application)?;
        Ok(
            rpc_proto::messages::hephaestus::common::v1::MutationReceipt {
                event_id: rpc_proto::messages::hephaestus::common::v1::OpaqueId {
                    value: row.event_id.to_string(),
                    ..Default::default()
                }
                .into(),
                committed_cursor: rpc_proto::messages::hephaestus::common::v1::Cursor {
                    value: self
                        .cursor_codec
                        .encode(&row.scope_kind, row.scope_id, row.cursor),
                    ..Default::default()
                }
                .into(),
                aggregate_version: u64::try_from(row.aggregate_version)
                    .map_err(|_| ConnectReceiptError::InvalidVersion)?,
                ..Default::default()
            },
        )
    }
}

#[derive(Debug, thiserror::Error)]
/// Errors that can occur while loading a committed mutation receipt.
pub enum ConnectReceiptError {
    #[error("mutation receipt application operation failed")]
    Application(#[source] event_application::MutationReceiptError),
    #[error("committed mutation event version is invalid")]
    InvalidVersion,
}

impl ConnectReceiptError {
    /// Returns the stable redacted class used in transport diagnostics.
    pub const fn error_class(&self) -> &'static str {
        match self {
            Self::Application(event_application::MutationReceiptError::Missing) => "missing",
            Self::Application(event_application::MutationReceiptError::Provider(_)) => {
                "provider-unavailable"
            }
            Self::InvalidVersion => "invalid-version",
        }
    }
}

/// Loads a mutation receipt and maps failures to a redacted Connect error.
pub async fn mutation_receipt(
    receipts: &MutationReceipts,
    occurrence_id: identity_domain::RequestId,
    actor_id: identity_domain::UserId,
    aggregate_type: &str,
    primary_scope_kind: &str,
) -> Result<rpc_proto::messages::hephaestus::common::v1::MutationReceipt, connectrpc::ConnectError>
{
    receipts
        .load(occurrence_id, actor_id, aggregate_type, primary_scope_kind)
        .await
        .map_err(|error| {
            tracing::error!(
                %occurrence_id,
                %aggregate_type,
                %primary_scope_kind,
                stage = "mutation-receipt",
                error_class = error.error_class(),
                "mutation receipt unavailable"
            );
            into_connect_error(RpcError::Internal)
        })
}

/// Derives the domain-separated HS256 key shared with the Phoenix mediator.
#[must_use]
pub fn mediator_signing_key(internal_token: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(MEDIATOR_KEY_DOMAIN);
    digest.update(internal_token);
    digest.finalize().into()
}
