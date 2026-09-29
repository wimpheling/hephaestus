//! Gateway mailbox publication port conversion and payload validation.

use super::GatewayEdgeError;
use super::gateway_mailbox_publisher::{
    GatewayMailboxPublicationRequest, GatewayMailboxPublicationResult,
    PostgresGatewayMailboxPublisher,
};
use async_trait::async_trait;
use gateway_domain::GatewayMailboxPublisher;
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, SelectedHeaderName, SelectedHeaderValue, TraceContext,
};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::PrivateMailboxPublication;

#[async_trait]
impl GatewayMailboxPublisher for PostgresGatewayMailboxPublisher {
    async fn publish(
        &self,
        invocation_id: Uuid,
        publication: PrivateMailboxPublication,
    ) -> Result<(), GatewayEdgeError> {
        let encoded_body = publication.body.to_vec();
        let byte_length =
            u32::try_from(encoded_body.len()).map_err(|_| GatewayEdgeError::Unavailable)?;
        let integrity_hash: [u8; 32] = Sha256::digest(&encoded_body).into();
        let body = BodyReference::new(BodyReferenceId::new(), byte_length, integrity_hash)
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        let content = ContentMetadata::new(
            body,
            publication.content_type,
            Some(String::from("identity")),
        )
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let headers = publication
            .headers
            .into_iter()
            .map(|(name, value)| {
                Ok((
                    SelectedHeaderName::parse(name).map_err(|_| GatewayEdgeError::Unavailable)?,
                    SelectedHeaderValue::parse(value).map_err(|_| GatewayEdgeError::Unavailable)?,
                ))
            })
            .collect::<Result<_, GatewayEdgeError>>()?;
        let envelope = MailboxEnvelope::new(
            EnvelopeMethod::parse(publication.method).map_err(|_| GatewayEdgeError::Unavailable)?,
            EnvelopeRoute::parse(publication.route).map_err(|_| GatewayEdgeError::Unavailable)?,
            headers,
            content,
            OffsetDateTime::now_utc(),
            publication
                .trace_context
                .map(TraceContext::parse)
                .transpose()
                .map_err(|_| GatewayEdgeError::Unavailable)?,
        )
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        // Gateway runtime session identity is deterministically the accepted
        // invocation identity. The publisher rechecks that exact session only
        // after entering its worker-role transaction; guests never provide it.
        let runtime_session_id = invocation_id;
        let result = self
            .publish(GatewayMailboxPublicationRequest {
                runtime_session_id,
                invocation_id,
                slot_key: publication.slot,
                deduplication_key: DeduplicationKey::parse(publication.deduplication_key)
                    .map_err(|_| GatewayEdgeError::Unavailable)?,
                envelope,
                encoded_body,
                decoded_length: byte_length,
            })
            .await?;
        match result {
            GatewayMailboxPublicationResult::Accepted { .. }
            | GatewayMailboxPublicationResult::Duplicate { .. } => Ok(()),
            GatewayMailboxPublicationResult::Denied => Err(GatewayEdgeError::HandlerUnavailable),
        }
    }
}

/// Validates encoded body evidence before the worker transaction starts.
pub fn validate_gateway_mailbox_payload(
    request: &GatewayMailboxPublicationRequest,
) -> Result<(), GatewayEdgeError> {
    let body = &request.envelope.content.body;
    if request.encoded_body.len()
        != usize::try_from(body.byte_length).map_err(|_| GatewayEdgeError::Unavailable)?
        || request.decoded_length != body.byte_length
        || Sha256::digest(&request.encoded_body).as_slice() != body.integrity_hash
        || request
            .envelope
            .content
            .content_encoding
            .as_deref()
            .is_some_and(|value| value != "identity")
    {
        return Err(GatewayEdgeError::Unavailable);
    }
    Ok(())
}
