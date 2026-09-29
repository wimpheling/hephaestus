//! `CloudEvent` header, body, and shape validation.

use crate::{
    credential::CallbackCredential,
    model::{
        NotificationIdempotencyKey, NotificationObservation, ObservedRepositoryPath, PayloadSha256,
        ZotEventType,
    },
    path::validate_reference,
    timestamp::parse_rfc3339,
};
use http::HeaderMap;
use registry_domain::{OciMediaType, Sha256Digest};
use serde::Deserialize;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const MAX_EVENT_AGE: Duration = Duration::days(7);
const MAX_FUTURE_SKEW: Duration = Duration::minutes(5);

/// Parses one authenticated Zot v2.1.18 binary-mode `CloudEvent`.
///
/// `received_at` is supplied by the transport adapter so tests and the caller
/// share one clock. Accepted callbacks are only durable observations; callers
/// must schedule authoritative Zot reconciliation before changing product
/// lifecycle state.
///
/// # Errors
///
/// Returns [`NotificationError`] for invalid callback credentials, `CloudEvent`
/// headers, timestamps, bounded body data, or unsupported Zot payloads.
pub fn parse_notification(
    headers: &HeaderMap,
    body: &[u8],
    callback_credential: &CallbackCredential,
    received_at: OffsetDateTime,
) -> Result<NotificationObservation, NotificationError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(NotificationError::BodyTooLarge);
    }

    let authorization = required_header(headers, "authorization")?;
    let Some(presented_credential) = authorization.strip_prefix("Bearer ") else {
        return Err(NotificationError::Unauthorized);
    };
    if !callback_credential.authenticates(presented_credential) {
        return Err(NotificationError::Unauthorized);
    }

    if required_header(headers, "content-type")? != "application/json" {
        return Err(NotificationError::InvalidContentType);
    }
    if required_header(headers, "ce-specversion")? != "1.0" {
        return Err(NotificationError::InvalidCloudEventSpecVersion);
    }

    let event_id = required_header(headers, "ce-id")?;
    let parsed_event_id =
        Uuid::parse_str(event_id).map_err(|_| NotificationError::InvalidEventId)?;
    if parsed_event_id.to_string() != event_id {
        return Err(NotificationError::InvalidEventId);
    }

    let source = required_header(headers, "ce-source")?;
    if source != "zotregistry.dev" {
        return Err(NotificationError::InvalidSource);
    }
    let event_type = ZotEventType::parse(required_header(headers, "ce-type")?)?;
    let occurred_at = parse_rfc3339(required_header(headers, "ce-time")?)?;
    if occurred_at < received_at - MAX_EVENT_AGE || occurred_at > received_at + MAX_FUTURE_SKEW {
        return Err(NotificationError::TimestampOutsideAcceptanceWindow);
    }

    let data: ZotEventData =
        serde_json::from_slice(body).map_err(|_| NotificationError::InvalidBody)?;
    validate_transient_context(data.actor.as_ref(), data.request.as_ref())?;
    let repository = ObservedRepositoryPath::parse(data.name)?;
    let reference = data.reference.map(validate_reference).transpose()?;
    let digest = data
        .digest
        .map(Sha256Digest::parse)
        .transpose()
        .map_err(|_| NotificationError::InvalidDigest)?;
    let media_type = data
        .media_type
        .map(OciMediaType::parse)
        .transpose()
        .map_err(|_| NotificationError::InvalidMediaType)?;

    validate_event_data(
        event_type,
        reference.as_ref(),
        digest.as_ref(),
        media_type.as_ref(),
        data.manifest.as_deref(),
    )?;

    Ok(NotificationObservation::new(
        NotificationIdempotencyKey::from_source_and_id(source, event_id),
        PayloadSha256::from_body(body),
        event_type,
        event_type.action(),
        repository,
        reference,
        digest,
        media_type,
        occurred_at,
        body.len(),
    ))
}

/// Notification validation failure with no secret or raw-payload text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NotificationError {
    /// The configured callback credential is not a safe generated token.
    #[error("invalid callback credential")]
    InvalidCallbackCredential,
    /// The request did not authenticate as the isolated Zot sink.
    #[error("unauthorized registry notification")]
    Unauthorized,
    /// A required callback header was absent.
    #[error("missing required registry notification header")]
    MissingHeader,
    /// A callback provided more than one value for a singular header.
    #[error("duplicate registry notification header")]
    DuplicateHeader,
    /// A callback header was not visible ASCII text.
    #[error("invalid registry notification header")]
    InvalidHeader,
    /// The callback body exceeded its explicit safety limit.
    #[error("registry notification body is too large")]
    BodyTooLarge,
    /// The Zot binary `CloudEvents` content type was not exact JSON.
    #[error("invalid registry notification content type")]
    InvalidContentType,
    /// The `CloudEvents` specification version was not supported.
    #[error("unsupported CloudEvents specification version")]
    InvalidCloudEventSpecVersion,
    /// The Zot event ID was not a canonical UUID.
    #[error("invalid registry notification event ID")]
    InvalidEventId,
    /// The `CloudEvent` source was not Zot's documented source identity.
    #[error("invalid registry notification source")]
    InvalidSource,
    /// Zot emitted an event category not handled by this control-plane version.
    #[error("unsupported registry notification event type")]
    UnsupportedEventType,
    /// The `CloudEvent` occurrence timestamp could not be parsed.
    #[error("invalid registry notification timestamp")]
    InvalidTimestamp,
    /// The timestamp was implausibly stale or too far in the future.
    #[error("registry notification timestamp is outside the acceptance window")]
    TimestampOutsideAcceptanceWindow,
    /// The binary `CloudEvent` JSON data did not match Zot's contract.
    #[error("invalid registry notification body")]
    InvalidBody,
    /// The Zot repository path was not bounded canonical OCI repository text.
    #[error("invalid registry notification repository path")]
    InvalidRepositoryPath,
    /// The Zot mutable manifest reference was not bounded canonical tag/digest text.
    #[error("invalid registry notification reference")]
    InvalidReference,
    /// The Zot digest was not a canonical SHA-256 digest.
    #[error("invalid registry notification digest")]
    InvalidDigest,
    /// The Zot media type was not a bounded OCI application media type.
    #[error("invalid registry notification media type")]
    InvalidMediaType,
    /// A Zot event omitted or included fields contrary to its documented shape.
    #[error("invalid registry notification event fields")]
    InvalidEventFields,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ZotEventData {
    name: String,
    #[serde(default)]
    reference: Option<String>,
    #[serde(default)]
    digest: Option<String>,
    #[serde(rename = "mediaType", default)]
    media_type: Option<String>,
    #[serde(default)]
    manifest: Option<String>,
    #[serde(default)]
    actor: Option<ActorMetadata>,
    #[serde(default)]
    request: Option<RequestMetadata>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActorMetadata {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestMetadata {
    addr: String,
    method: String,
    useragent: String,
}

fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, NotificationError> {
    let values = headers.get_all(name);
    let mut values = values.iter();
    let Some(value) = values.next() else {
        return Err(NotificationError::MissingHeader);
    };
    if values.next().is_some() {
        return Err(NotificationError::DuplicateHeader);
    }
    value.to_str().map_err(|_| NotificationError::InvalidHeader)
}

const fn validate_event_data(
    event_type: ZotEventType,
    reference: Option<&String>,
    digest: Option<&Sha256Digest>,
    media_type: Option<&OciMediaType>,
    manifest: Option<&str>,
) -> Result<(), NotificationError> {
    let image_event = matches!(
        event_type,
        ZotEventType::ImageUpdated | ZotEventType::ImageDeleted | ZotEventType::ImageLintFailed
    );
    if image_event && (reference.is_none() || digest.is_none() || media_type.is_none()) {
        return Err(NotificationError::InvalidEventFields);
    }

    match event_type {
        ZotEventType::RepositoryCreated => {
            if reference.is_some() || digest.is_some() || media_type.is_some() || manifest.is_some()
            {
                return Err(NotificationError::InvalidEventFields);
            }
        }
        ZotEventType::ImageUpdated | ZotEventType::ImageLintFailed => {
            if manifest.is_none() {
                return Err(NotificationError::InvalidEventFields);
            }
        }
        ZotEventType::ImageDeleted if manifest.is_some() => {
            return Err(NotificationError::InvalidEventFields);
        }
        ZotEventType::ImageDeleted => {}
    }
    Ok(())
}

fn validate_transient_context(
    actor: Option<&ActorMetadata>,
    request: Option<&RequestMetadata>,
) -> Result<(), NotificationError> {
    let valid_text = |value: &str, maximum: usize| {
        !value.is_empty()
            && value.len() <= maximum
            && value
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' ')
    };
    if actor.is_some_and(|actor| !valid_text(&actor.name, 256))
        || request.is_some_and(|request| {
            !valid_text(&request.addr, 255)
                || !valid_text(&request.method, 16)
                || !valid_text(&request.useragent, 512)
        })
    {
        return Err(NotificationError::InvalidBody);
    }
    Ok(())
}
