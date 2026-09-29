use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::OffsetDateTime;

use super::{
    BODY_INTEGRITY_HASH_BYTES, BodyReferenceId, MAX_BODY_BYTES, MAX_CONTENT_METADATA_BYTES,
    MAX_HEADERS,
    errors::MailboxDomainError,
    identifiers::{EnvelopeMethod, validate_opaque},
    routing::{EnvelopeRoute, SelectedHeaderName, SelectedHeaderValue, TraceContext},
};

/// Exact integrity metadata for an opaque accepted body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyReference {
    /// Opaque body identity; commands carry this rather than payload bytes.
    pub id: BodyReferenceId,
    /// Exact stored body length.
    pub byte_length: u32,
    /// SHA-256 integrity hash of the stored bytes.
    pub integrity_hash: [u8; BODY_INTEGRITY_HASH_BYTES],
}

impl BodyReference {
    /// Creates validated body metadata.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::BodyTooLarge`] when the supplied size
    /// exceeds the platform's bounded mailbox body limit.
    pub const fn new(
        id: BodyReferenceId,
        byte_length: u32,
        integrity_hash: [u8; BODY_INTEGRITY_HASH_BYTES],
    ) -> Result<Self, MailboxDomainError> {
        if byte_length > MAX_BODY_BYTES {
            return Err(MailboxDomainError::BodyTooLarge);
        }
        Ok(Self {
            id,
            byte_length,
            integrity_hash,
        })
    }
}

/// Bounded, opaque content metadata attached to a mailbox body reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentMetadata {
    /// Referenced payload, never inline payload bytes.
    pub body: BodyReference,
    /// Declared content type, if one was supplied.
    pub content_type: Option<String>,
    /// Declared content encoding, if one was supplied.
    pub content_encoding: Option<String>,
}

impl ContentMetadata {
    /// Creates bounded content metadata.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidContentMetadata`] for blank,
    /// oversized, or control-character metadata.
    pub fn new(
        body: BodyReference,
        content_type: Option<String>,
        content_encoding: Option<String>,
    ) -> Result<Self, MailboxDomainError> {
        for value in [content_type.as_deref(), content_encoding.as_deref()]
            .into_iter()
            .flatten()
        {
            validate_opaque(
                value,
                MAX_CONTENT_METADATA_BYTES,
                MailboxDomainError::InvalidContentMetadata,
            )?;
        }
        Ok(Self {
            body,
            content_type,
            content_encoding,
        })
    }
}
/// Bounded provider-neutral metadata accepted for one mailbox event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxEnvelope {
    /// Opaque transport method.
    pub method: EnvelopeMethod,
    /// Opaque bounded route.
    pub route: EnvelopeRoute,
    /// Allowlisted selected headers, sorted by canonical lower-case name.
    pub headers: BTreeMap<SelectedHeaderName, SelectedHeaderValue>,
    /// Payload metadata and opaque reference.
    pub content: ContentMetadata,
    /// Platform receive timestamp.
    pub received_at: OffsetDateTime,
    /// Optional opaque trace context.
    pub trace_context: Option<TraceContext>,
}

impl MailboxEnvelope {
    /// Creates a bounded envelope.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::TooManyHeaders`] when more than the
    /// bounded selected-header allowance is supplied.
    pub fn new(
        method: EnvelopeMethod,
        route: EnvelopeRoute,
        headers: BTreeMap<SelectedHeaderName, SelectedHeaderValue>,
        content: ContentMetadata,
        received_at: OffsetDateTime,
        trace_context: Option<TraceContext>,
    ) -> Result<Self, MailboxDomainError> {
        if headers.len() > MAX_HEADERS {
            return Err(MailboxDomainError::TooManyHeaders);
        }
        Ok(Self {
            method,
            route,
            headers,
            content,
            received_at,
            trace_context,
        })
    }
}
