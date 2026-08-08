//! Private authenticated HTTP ingestion for Zot `CloudEvents` callbacks.

use async_trait::async_trait;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use registry_notification::{
    CallbackCredential, NotificationError, NotificationObservation, parse_notification,
};
use serde::Serialize;
use std::sync::Arc;
use time::OffsetDateTime;

/// Private callback path configured in Zot's event sink.
pub const REGISTRY_NOTIFICATION_PATH: &str = "/internal/v1/registry/notifications";

const MAX_NOTIFICATION_BODY_BYTES: usize = 5 * 1024 * 1024;

/// Durable inbox boundary for validated notification observations.
#[async_trait]
pub trait RegistryNotificationInbox: Send + Sync + 'static {
    /// Idempotently records one bounded observation before returning success.
    ///
    /// # Errors
    ///
    /// Returns a non-sensitive provider failure. The caller returns a retryable
    /// response, although authoritative reconciliation remains required because
    /// Zot's HTTP sink itself is best effort.
    async fn ingest(
        &self,
        observation: NotificationObservation,
    ) -> Result<InboxDisposition, RegistryInboxError>;
}

/// Result of an idempotent inbox insert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxDisposition {
    /// A new durable inbox row was committed.
    Accepted,
    /// The same Zot event was already durable.
    Duplicate,
}

/// Non-sensitive notification inbox provider failure.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("registry notification inbox is unavailable")]
pub struct RegistryInboxError;

/// Configured private notification endpoint.
#[derive(Clone)]
pub struct RegistryNotificationHttpService {
    credential: CallbackCredential,
    inbox: Arc<dyn RegistryNotificationInbox>,
}

impl RegistryNotificationHttpService {
    /// Creates the endpoint with one callback verifier and durable inbox.
    #[must_use]
    pub fn new(credential: CallbackCredential, inbox: Arc<dyn RegistryNotificationInbox>) -> Self {
        Self { credential, inbox }
    }

    /// Builds the one-route private ingestion service with an explicit body
    /// limit matching the domain parser.
    pub fn router(self) -> Router {
        Router::new()
            .route(REGISTRY_NOTIFICATION_PATH, post(ingest_notification))
            .layer(DefaultBodyLimit::max(MAX_NOTIFICATION_BODY_BYTES))
            .with_state(Arc::new(self))
    }

    async fn ingest(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        received_at: OffsetDateTime,
    ) -> Result<InboxDisposition, NotificationHttpError> {
        let observation = parse_notification(headers, body, &self.credential, received_at)
            .map_err(NotificationHttpError::InvalidNotification)?;
        self.inbox
            .ingest(observation)
            .await
            .map_err(|_| NotificationHttpError::InboxUnavailable)
    }
}

async fn ingest_notification(
    State(service): State<Arc<RegistryNotificationHttpService>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match service
        .ingest(&headers, &body, OffsetDateTime::now_utc())
        .await
    {
        Ok(disposition) => (
            StatusCode::ACCEPTED,
            Json(AcceptedResponse {
                duplicate: disposition == InboxDisposition::Duplicate,
            }),
        )
            .into_response(),
        Err(error) => error.into_response(),
    }
}

#[derive(Debug)]
enum NotificationHttpError {
    InvalidNotification(NotificationError),
    InboxUnavailable,
}

impl IntoResponse for NotificationHttpError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::InvalidNotification(NotificationError::Unauthorized) => {
                (StatusCode::UNAUTHORIZED, "unauthorized")
            }
            Self::InvalidNotification(NotificationError::BodyTooLarge) => {
                (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large")
            }
            Self::InvalidNotification(_) => (StatusCode::BAD_REQUEST, "invalid_notification"),
            Self::InboxUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "inbox_unavailable"),
        };
        tracing::warn!(code, "registry notification rejected");
        (status, Json(ErrorResponse { code })).into_response()
    }
}

#[derive(Serialize)]
struct AcceptedResponse {
    duplicate: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    code: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;
    use std::sync::Mutex;

    const CALLBACK_TOKEN: &str = "0123456789abcdefghi_jklmnopqrstuvwxyz-ABCDEFG";
    const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[derive(Default)]
    struct MemoryInbox(Mutex<Vec<NotificationObservation>>);

    #[async_trait]
    impl RegistryNotificationInbox for MemoryInbox {
        async fn ingest(
            &self,
            observation: NotificationObservation,
        ) -> Result<InboxDisposition, RegistryInboxError> {
            let mut rows = self.0.lock().expect("memory inbox lock");
            let duplicate = rows.iter().any(|row| {
                row.idempotency_key() == observation.idempotency_key()
                    && row.payload_sha256() == observation.payload_sha256()
            });
            if duplicate {
                drop(rows);
                Ok(InboxDisposition::Duplicate)
            } else {
                rows.push(observation);
                drop(rows);
                Ok(InboxDisposition::Accepted)
            }
        }
    }

    fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer 0123456789abcdefghi_jklmnopqrstuvwxyz-ABCDEFG"),
        );
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        headers.insert("ce-specversion", HeaderValue::from_static("1.0"));
        headers.insert(
            "ce-id",
            HeaderValue::from_static("a8098c1a-f86e-11da-bd1a-00112444be1e"),
        );
        headers.insert("ce-source", HeaderValue::from_static("zotregistry.dev"));
        headers.insert(
            "ce-type",
            HeaderValue::from_static("zotregistry.image.updated"),
        );
        headers.insert("ce-time", HeaderValue::from_static("2026-08-04T12:00:00Z"));
        headers
    }

    fn body() -> String {
        format!(
            r#"{{"name":"platform/builders/rust-ubuntu","reference":"latest","digest":"{DIGEST}","mediaType":"application/vnd.oci.image.manifest.v1+json","manifest":"{{}}"}}"#
        )
    }

    #[tokio::test]
    async fn authenticated_duplicate_is_acknowledged_idempotently() {
        let inbox = Arc::new(MemoryInbox::default());
        let service = RegistryNotificationHttpService::new(
            CallbackCredential::parse(CALLBACK_TOKEN).expect("credential"),
            inbox,
        );
        let received_at = OffsetDateTime::from_unix_timestamp(1_785_844_801).expect("time");

        assert_eq!(
            service
                .ingest(&headers(), body().as_bytes(), received_at)
                .await
                .expect("first event"),
            InboxDisposition::Accepted
        );
        assert_eq!(
            service
                .ingest(&headers(), body().as_bytes(), received_at)
                .await
                .expect("duplicate event"),
            InboxDisposition::Duplicate
        );
    }

    #[tokio::test]
    async fn forged_callback_never_reaches_the_inbox() {
        let inbox = Arc::new(MemoryInbox::default());
        let service = RegistryNotificationHttpService::new(
            CallbackCredential::parse(CALLBACK_TOKEN).expect("credential"),
            Arc::clone(&inbox) as Arc<dyn RegistryNotificationInbox>,
        );
        let mut forged = headers();
        forged.insert(
            "authorization",
            HeaderValue::from_static("Bearer AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        );

        assert!(
            service
                .ingest(
                    &forged,
                    body().as_bytes(),
                    OffsetDateTime::from_unix_timestamp(1_785_844_801).expect("time"),
                )
                .await
                .is_err()
        );
        assert!(inbox.0.lock().expect("memory inbox lock").is_empty());
    }
}
