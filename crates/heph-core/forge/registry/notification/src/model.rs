//! Validated notification metadata and public domain values.

use crate::{parser::NotificationError, path::is_canonical_repository_path};
use registry_domain::{OciMediaType, RegistryNamespace, Sha256Digest};
use sha2::{Digest, Sha256};
use std::fmt;
use time::OffsetDateTime;

/// The semantic action represented by a Zot notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationAction {
    /// Zot created a repository, updated a manifest, or reported lint failure.
    Push,
    /// Zot deleted a manifest reference.
    Delete,
}

/// The exact Zot event category that produced an inbox observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZotEventType {
    /// A repository was first created.
    RepositoryCreated,
    /// An image manifest was stored or its reference was updated.
    ImageUpdated,
    /// An image manifest reference was deleted.
    ImageDeleted,
    /// Zot's lint extension reported a failed image lint.
    ImageLintFailed,
}

impl ZotEventType {
    /// Parses a documented Zot event type.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported event type.
    pub fn parse(value: &str) -> Result<Self, NotificationError> {
        match value {
            "zotregistry.repository.created" => Ok(Self::RepositoryCreated),
            "zotregistry.image.updated" => Ok(Self::ImageUpdated),
            "zotregistry.image.deleted" => Ok(Self::ImageDeleted),
            "zotregistry.image.lint_failed" => Ok(Self::ImageLintFailed),
            _ => Err(NotificationError::UnsupportedEventType),
        }
    }

    /// Returns the action stored with the observation.
    #[must_use]
    pub const fn action(self) -> NotificationAction {
        match self {
            Self::ImageDeleted => NotificationAction::Delete,
            Self::RepositoryCreated | Self::ImageUpdated | Self::ImageLintFailed => {
                NotificationAction::Push
            }
        }
    }
}

/// A stable hexadecimal SHA-256 idempotency key derived from `CloudEvent` source
/// and ID.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NotificationIdempotencyKey(String);

impl NotificationIdempotencyKey {
    /// Derives the stable idempotency key from `CloudEvent` identity.
    #[must_use]
    pub fn from_source_and_id(source: &str, event_id: &str) -> Self {
        let mut digest = Sha256::new();
        digest.update(source.as_bytes());
        digest.update([0]);
        digest.update(event_id.as_bytes());

        Self(hex_encode(&digest.finalize()))
    }

    /// Returns the fixed-width lowercase hexadecimal key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The SHA-256 hash of the exact HTTP body, without retaining the body.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PayloadSha256([u8; 32]);

impl PayloadSha256 {
    /// Hashes the exact callback body.
    #[must_use]
    pub fn from_body(body: &[u8]) -> Self {
        Self(Sha256::digest(body).into())
    }

    /// Returns the hash bytes for a `bytea` durable-inbox column.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the lowercase hexadecimal representation for diagnostics.
    #[must_use]
    pub fn to_hex(self) -> String {
        hex_encode(&self.0)
    }
}

impl fmt::Debug for PayloadSha256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("PayloadSha256")
            .field(&self.to_hex())
            .finish()
    }
}

/// A bounded repository path observed from Zot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedRepositoryPath {
    value: String,
    known_namespace: Option<RegistryNamespace>,
}

impl ObservedRepositoryPath {
    /// Parses a bounded repository path and optional owned namespace.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is not canonical bounded OCI text.
    pub fn parse(value: String) -> Result<Self, NotificationError> {
        if !is_canonical_repository_path(&value) {
            return Err(NotificationError::InvalidRepositoryPath);
        }

        Ok(Self {
            known_namespace: RegistryNamespace::parse(value.clone()).ok(),
            value,
        })
    }

    /// Returns the canonical path supplied by Zot.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Returns the Hephaestus-owned namespace when the path is recognized.
    ///
    /// Unknown but syntactically valid paths are retained as bounded metadata so
    /// reconciliation can safely surface unauthorized/orphaned Zot content.
    #[must_use]
    pub const fn known_namespace(&self) -> Option<&RegistryNamespace> {
        self.known_namespace.as_ref()
    }
}

/// Bounded metadata ready for an idempotent registry-notification inbox row.
///
/// No raw body, manifest, actor, user-agent, address, authorization header, or
/// callback credential is retained by this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationObservation {
    idempotency_key: NotificationIdempotencyKey,
    payload_sha256: PayloadSha256,
    event_type: ZotEventType,
    action: NotificationAction,
    repository: ObservedRepositoryPath,
    reference: Option<String>,
    digest: Option<Sha256Digest>,
    media_type: Option<OciMediaType>,
    occurred_at: OffsetDateTime,
    body_size: usize,
}

impl NotificationObservation {
    /// Builds validated metadata from the parser's bounded components.
    #[must_use]
    // The parser produces these fixed fields independently while preserving
    // the private representation of the public observation type.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        idempotency_key: NotificationIdempotencyKey,
        payload_sha256: PayloadSha256,
        event_type: ZotEventType,
        action: NotificationAction,
        repository: ObservedRepositoryPath,
        reference: Option<String>,
        digest: Option<Sha256Digest>,
        media_type: Option<OciMediaType>,
        occurred_at: OffsetDateTime,
        body_size: usize,
    ) -> Self {
        Self {
            idempotency_key,
            payload_sha256,
            event_type,
            action,
            repository,
            reference,
            digest,
            media_type,
            occurred_at,
            body_size,
        }
    }

    /// Returns the durable idempotency key.
    #[must_use]
    pub const fn idempotency_key(&self) -> &NotificationIdempotencyKey {
        &self.idempotency_key
    }

    /// Returns the exact body hash for deduplication diagnostics.
    #[must_use]
    pub const fn payload_sha256(&self) -> &PayloadSha256 {
        &self.payload_sha256
    }

    /// Returns Zot's event type.
    #[must_use]
    pub const fn event_type(&self) -> ZotEventType {
        self.event_type
    }

    /// Returns the compact inbox action.
    #[must_use]
    pub const fn action(&self) -> NotificationAction {
        self.action
    }

    /// Returns the observed repository route.
    #[must_use]
    pub const fn repository(&self) -> &ObservedRepositoryPath {
        &self.repository
    }

    /// Returns the bounded mutable Zot reference, if the event has one.
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }

    /// Returns the observed immutable manifest digest, if the event has one.
    #[must_use]
    pub const fn digest(&self) -> Option<&Sha256Digest> {
        self.digest.as_ref()
    }

    /// Returns the observed manifest media type, if the event has one.
    #[must_use]
    pub const fn media_type(&self) -> Option<&OciMediaType> {
        self.media_type.as_ref()
    }

    /// Returns the validated body size. This is not an OCI descriptor size.
    #[must_use]
    pub const fn body_size(&self) -> usize {
        self.body_size
    }

    /// Returns the `CloudEvent` occurrence timestamp.
    #[must_use]
    pub const fn occurred_at(&self) -> OffsetDateTime {
        self.occurred_at
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}
