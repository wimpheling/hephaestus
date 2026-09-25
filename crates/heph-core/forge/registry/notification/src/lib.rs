//! Authenticated, bounded Zot event notification ingestion.
//!
//! Zot v2.1.18 sends registry events as binary-mode `CloudEvents` over HTTP. This
//! crate validates that transport contract and returns only metadata suitable
//! for a durable inbox. It intentionally owns neither publication approval nor
//! product-event emission: a callback only requests reconciliation.

use hmac::{Hmac, Mac};
use http::HeaderMap;
use registry_domain::{OciMediaType, RegistryNamespace, Sha256Digest};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};
use time::{Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};
use uuid::Uuid;

const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const MAX_EVENT_AGE: Duration = Duration::days(7);
const MAX_FUTURE_SKEW: Duration = Duration::minutes(5);
const CALLBACK_CREDENTIAL_MINIMUM_LENGTH: usize = 43;
const CALLBACK_CREDENTIAL_MAXIMUM_LENGTH: usize = 128;
const CALLBACK_CREDENTIAL_CONTEXT: &[u8] = b"hephaestus/zot-notification-callback/v1";

type HmacSha256 = Hmac<Sha256>;

/// A private callback credential configured in Zot's HTTP event sink.
///
/// The constructor accepts only an unpadded URL-safe base64 token encoding at
/// least 256 bits of entropy. The value is immediately reduced to a verifier;
/// neither the original credential nor its derived verifier are exposed by
/// [`Debug`](fmt::Debug).
#[derive(Clone, PartialEq, Eq)]
pub struct CallbackCredential([u8; 32]);

impl CallbackCredential {
    /// Parses a generated private callback token.
    ///
    /// # Errors
    ///
    /// Returns [`NotificationError::InvalidCallbackCredential`] if the value
    /// cannot safely serve as a high-entropy bearer credential.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, NotificationError> {
        let value = value.as_ref();
        let valid = (CALLBACK_CREDENTIAL_MINIMUM_LENGTH..=CALLBACK_CREDENTIAL_MAXIMUM_LENGTH)
            .contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if !valid {
            return Err(NotificationError::InvalidCallbackCredential);
        }

        Ok(Self(callback_verifier(value.as_bytes())))
    }

    fn authenticates(&self, presented: &str) -> bool {
        if !(CALLBACK_CREDENTIAL_MINIMUM_LENGTH..=CALLBACK_CREDENTIAL_MAXIMUM_LENGTH)
            .contains(&presented.len())
            || !presented
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return false;
        }

        // `verify_slice` performs a constant-time comparison. The configured
        // token itself is never retained after construction.
        HmacSha256::new_from_slice(CALLBACK_CREDENTIAL_CONTEXT)
            .expect("the fixed callback credential context is a valid HMAC key")
            .chain_update(presented.as_bytes())
            .verify_slice(&self.0)
            .is_ok()
    }
}

impl fmt::Debug for CallbackCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CallbackCredential([REDACTED])")
    }
}

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
    fn parse(value: &str) -> Result<Self, NotificationError> {
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
    fn from_source_and_id(source: &str, event_id: &str) -> Self {
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
    fn from_body(body: &[u8]) -> Self {
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
    fn parse(value: String) -> Result<Self, NotificationError> {
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

    Ok(NotificationObservation {
        idempotency_key: NotificationIdempotencyKey::from_source_and_id(source, event_id),
        payload_sha256: PayloadSha256::from_body(body),
        action: event_type.action(),
        event_type,
        repository,
        reference,
        digest,
        media_type,
        occurred_at,
        body_size: body.len(),
    })
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

fn callback_verifier(value: &[u8]) -> [u8; 32] {
    HmacSha256::new_from_slice(CALLBACK_CREDENTIAL_CONTEXT)
        .expect("the fixed callback credential context is a valid HMAC key")
        .chain_update(value)
        .finalize()
        .into_bytes()
        .into()
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

fn parse_rfc3339(value: &str) -> Result<OffsetDateTime, NotificationError> {
    let Some((date, time_and_offset)) = value.split_once('T') else {
        return Err(NotificationError::InvalidTimestamp);
    };
    let mut date_parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) = (
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
    ) else {
        return Err(NotificationError::InvalidTimestamp);
    };
    let year = parse_fixed_decimal::<i32>(year, 4)?;
    let month = Month::try_from(parse_fixed_decimal::<u8>(month, 2)?)
        .map_err(|_| NotificationError::InvalidTimestamp)?;
    let day = parse_fixed_decimal::<u8>(day, 2)?;
    let date = Date::from_calendar_date(year, month, day)
        .map_err(|_| NotificationError::InvalidTimestamp)?;

    let (time, offset) = if let Some(time) = time_and_offset.strip_suffix('Z') {
        (time, UtcOffset::UTC)
    } else {
        let Some(offset_index) = time_and_offset
            .char_indices()
            .skip(8)
            .find_map(|(index, character)| matches!(character, '+' | '-').then_some(index))
        else {
            return Err(NotificationError::InvalidTimestamp);
        };
        let (time, offset) = time_and_offset.split_at(offset_index);
        (time, parse_utc_offset(offset)?)
    };
    let time = parse_time(time)?;

    Ok(PrimitiveDateTime::new(date, time).assume_offset(offset))
}

fn parse_time(value: &str) -> Result<Time, NotificationError> {
    let (whole_seconds, fractional) = match value.split_once('.') {
        Some((whole_seconds, fractional)) => (whole_seconds, Some(fractional)),
        None => (value, None),
    };
    let mut time_parts = whole_seconds.split(':');
    let (Some(hour), Some(minute), Some(second), None) = (
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
    ) else {
        return Err(NotificationError::InvalidTimestamp);
    };
    let nanosecond = fractional.map_or(Ok(0), parse_nanosecond)?;
    Time::from_hms_nano(
        parse_fixed_decimal::<u8>(hour, 2)?,
        parse_fixed_decimal::<u8>(minute, 2)?,
        parse_fixed_decimal::<u8>(second, 2)?,
        nanosecond,
    )
    .map_err(|_| NotificationError::InvalidTimestamp)
}

fn parse_utc_offset(value: &str) -> Result<UtcOffset, NotificationError> {
    if value.len() != 6
        || !matches!(value.as_bytes().first(), Some(b'+' | b'-'))
        || value.as_bytes()[3] != b':'
    {
        return Err(NotificationError::InvalidTimestamp);
    }
    let sign = if value.starts_with('-') { -1 } else { 1 };
    let hours = parse_fixed_decimal::<i8>(&value[1..3], 2)?;
    let minutes = parse_fixed_decimal::<i8>(&value[4..6], 2)?;
    UtcOffset::from_hms(sign * hours, sign * minutes, 0)
        .map_err(|_| NotificationError::InvalidTimestamp)
}

fn parse_nanosecond(value: &str) -> Result<u32, NotificationError> {
    if value.is_empty() || value.len() > 9 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NotificationError::InvalidTimestamp);
    }
    let parsed = value
        .parse::<u32>()
        .map_err(|_| NotificationError::InvalidTimestamp)?;
    let scale = 9_u32
        .checked_sub(u32::try_from(value.len()).map_err(|_| NotificationError::InvalidTimestamp)?)
        .ok_or(NotificationError::InvalidTimestamp)?;
    Ok(parsed * 10_u32.pow(scale))
}

fn parse_fixed_decimal<T>(value: &str, length: usize) -> Result<T, NotificationError>
where
    T: FromStr,
{
    if value.len() != length || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NotificationError::InvalidTimestamp);
    }
    value
        .parse()
        .map_err(|_| NotificationError::InvalidTimestamp)
}

fn is_canonical_repository_path(value: &str) -> bool {
    (1..=255).contains(&value.len())
        && value.split('/').all(|component| {
            !component.is_empty()
                && component.len() <= 128
                && component.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
                })
                && !component.ends_with(['.', '_', '-'])
        })
}

fn validate_reference(value: String) -> Result<String, NotificationError> {
    let is_digest = Sha256Digest::from_str(&value).is_ok();
    let is_tag = (1..=128).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric()
                || byte == b'_'
                || (index > 0 && matches!(byte, b'.' | b'-'))
        });
    (is_digest || is_tag)
        .then_some(value)
        .ok_or(NotificationError::InvalidReference)
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

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderValue};

    const CALLBACK_TOKEN: &str = "0123456789abcdefghi_jklmnopqrstuvwxyz-ABCDEFG";
    const EVENT_ID: &str = "a8098c1a-f86e-11da-bd1a-00112444be1e";
    const EVENT_TIME: &str = "2026-08-04T12:00:00Z";
    const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer 0123456789abcdefghi_jklmnopqrstuvwxyz-ABCDEFG"),
        );
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        headers.insert("ce-specversion", HeaderValue::from_static("1.0"));
        headers.insert("ce-id", HeaderValue::from_static(EVENT_ID));
        headers.insert("ce-source", HeaderValue::from_static("zotregistry.dev"));
        headers.insert(
            "ce-type",
            HeaderValue::from_static("zotregistry.image.updated"),
        );
        headers.insert("ce-time", HeaderValue::from_static(EVENT_TIME));
        headers
    }

    fn image_updated_body() -> String {
        format!(
            r#"{{"name":"platform/images/rust-ubuntu","reference":"latest","digest":"{DIGEST}","mediaType":"application/vnd.oci.image.manifest.v1+json","manifest":"{{}}","actor":{{"name":"worker"}},"request":{{"addr":"10.0.0.2:1234","method":"PUT","useragent":"skopeo"}}}}"#
        )
    }

    fn parse(body: &str) -> Result<NotificationObservation, NotificationError> {
        parse_notification(
            &headers(),
            body.as_bytes(),
            &CallbackCredential::parse(CALLBACK_TOKEN).expect("credential"),
            received_at(),
        )
    }

    fn received_at() -> OffsetDateTime {
        parse_rfc3339("2026-08-04T12:00:01Z").expect("fixed timestamp")
    }

    #[test]
    fn parses_zot_image_updated_without_retaining_sensitive_or_raw_metadata() {
        let body = image_updated_body();
        let observation = parse(&body).expect("valid Zot event");

        assert_eq!(observation.action(), NotificationAction::Push);
        assert_eq!(observation.event_type(), ZotEventType::ImageUpdated);
        assert_eq!(
            observation.repository().as_str(),
            "platform/images/rust-ubuntu"
        );
        assert!(observation.repository().known_namespace().is_some());
        assert_eq!(observation.reference(), Some("latest"));
        assert_eq!(observation.digest().map(Sha256Digest::as_str), Some(DIGEST));
        assert_eq!(observation.body_size(), body.len());
        assert_eq!(observation.idempotency_key().as_str().len(), 64);
        assert_eq!(observation.payload_sha256().as_bytes().len(), 32);
        let debug = format!("{observation:?}");
        assert!(!debug.contains(CALLBACK_TOKEN));
        assert!(!debug.contains("10.0.0.2"));
        assert!(!debug.contains("skopeo"));
    }

    #[test]
    fn duplicate_delivery_derives_the_same_key_and_body_hash() {
        let body = image_updated_body();
        let first = parse(&body).expect("first delivery");
        let second = parse(&body).expect("duplicate delivery");

        assert_eq!(first.idempotency_key(), second.idempotency_key());
        assert_eq!(first.payload_sha256(), second.payload_sha256());
    }

    #[test]
    fn rejects_forged_callback_without_disclosing_the_configured_credential() {
        let mut forged = headers();
        forged.insert(
            "authorization",
            HeaderValue::from_static("Bearer AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        );
        let credential = CallbackCredential::parse(CALLBACK_TOKEN).expect("credential");
        let error = parse_notification(
            &forged,
            image_updated_body().as_bytes(),
            &credential,
            received_at(),
        )
        .expect_err("forged callback must fail");

        assert_eq!(error, NotificationError::Unauthorized);
        assert!(!format!("{credential:?} {error:?}").contains(CALLBACK_TOKEN));
    }

    #[test]
    fn accepts_unknown_but_canonical_paths_for_safe_reconciliation_diagnostics() {
        let body = image_updated_body().replace(
            "platform/images/rust-ubuntu",
            "projects/unknown/repository-images/orphan",
        );
        let observation = parse(&body).expect("bounded unknown namespace is observable");

        assert!(observation.repository().known_namespace().is_none());
        assert_eq!(
            observation.repository().as_str(),
            "projects/unknown/repository-images/orphan"
        );
    }

    #[test]
    fn rejects_malformed_or_incomplete_zot_event_data() {
        let body = format!(
            r#"{{"name":"platform/builders/rust-ubuntu","reference":"latest","digest":"{DIGEST}","mediaType":"application/vnd.oci.image.manifest.v1+json"}}"#
        );
        assert_eq!(parse(&body), Err(NotificationError::InvalidEventFields));
    }

    #[test]
    fn rejects_wrong_source_stale_time_and_duplicate_headers() {
        let credential = CallbackCredential::parse(CALLBACK_TOKEN).expect("credential");
        let mut wrong_source = headers();
        wrong_source.insert("ce-source", HeaderValue::from_static("attacker.invalid"));
        assert_eq!(
            parse_notification(
                &wrong_source,
                image_updated_body().as_bytes(),
                &credential,
                received_at(),
            ),
            Err(NotificationError::InvalidSource)
        );

        let mut stale = headers();
        stale.insert("ce-time", HeaderValue::from_static("2026-07-01T12:00:00Z"));
        assert_eq!(
            parse_notification(
                &stale,
                image_updated_body().as_bytes(),
                &credential,
                received_at(),
            ),
            Err(NotificationError::TimestampOutsideAcceptanceWindow)
        );

        let mut duplicate = headers();
        duplicate.append(
            "ce-type",
            HeaderValue::from_static("zotregistry.image.updated"),
        );
        assert_eq!(
            parse_notification(
                &duplicate,
                image_updated_body().as_bytes(),
                &credential,
                received_at(),
            ),
            Err(NotificationError::DuplicateHeader)
        );
    }

    #[test]
    fn validates_all_documented_zot_event_shapes() {
        let credential = CallbackCredential::parse(CALLBACK_TOKEN).expect("credential");

        let repository_created = r#"{"name":"platform/builders/ubuntu-native"}"#;
        let mut created_headers = headers();
        created_headers.insert(
            "ce-type",
            HeaderValue::from_static("zotregistry.repository.created"),
        );
        assert_eq!(
            parse_notification(
                &created_headers,
                repository_created.as_bytes(),
                &credential,
                received_at(),
            )
            .expect("repository-created event")
            .action(),
            NotificationAction::Push
        );

        let deleted = format!(
            r#"{{"name":"platform/builders/ubuntu-native","reference":"latest","digest":"{DIGEST}","mediaType":"application/vnd.oci.image.manifest.v1+json"}}"#
        );
        let mut deleted_headers = headers();
        deleted_headers.insert(
            "ce-type",
            HeaderValue::from_static("zotregistry.image.deleted"),
        );
        assert_eq!(
            parse_notification(
                &deleted_headers,
                deleted.as_bytes(),
                &credential,
                received_at(),
            )
            .expect("image-deleted event")
            .action(),
            NotificationAction::Delete
        );

        let mut lint_headers = headers();
        lint_headers.insert(
            "ce-type",
            HeaderValue::from_static("zotregistry.image.lint_failed"),
        );
        assert_eq!(
            parse_notification(
                &lint_headers,
                image_updated_body().as_bytes(),
                &credential,
                received_at(),
            )
            .expect("image-lint-failed event")
            .event_type(),
            ZotEventType::ImageLintFailed
        );
    }
}
