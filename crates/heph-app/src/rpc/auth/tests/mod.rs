use super::{
    BOOTSTRAP_ACTOR_KIND, BOOTSTRAP_AUDIENCE, BOOTSTRAP_SUBJECT, BootstrapIdentity,
    CLOCK_SKEW_SECONDS, CREATE_BOOTSTRAP_AUDIENCE, HANDOFF_AUDIENCE, ISSUER,
    MediatorAuthenticationState, MediatorAuthenticator, REVOKE_AUDIENCE,
    mediator_identity_middleware, requires_mediator_auth,
};
use crate::rpc::mediator_signing_key;
use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
use identity_application::{
    BrowserSessionAuthenticationError, BrowserSessionStore, CreateBrowserSession,
    CreateBrowserSessionError, CreatedBrowserSession, RevokeBrowserSession,
    RevokeBrowserSessionError, RevokedBrowserSession,
};
use identity_domain::{BrowserSessionId, BrowserSessionMetadata, BrowserSessionSid, UserId};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use release_service::{
    NewUiRequestAuditEvent, UiRequestAuditDecision, UiRequestAuditOutcome, UiRequestAuditReason,
    UiRequestAuditSink, UiRequestAuditSurface,
};
use serde::Serialize;
use std::{
    future::pending,
    sync::{Arc, Mutex},
};
use time::{Duration, OffsetDateTime};
use tower::ServiceExt;
use uuid::Uuid;

const TOKEN: &[u8] = b"test-mediator-token-with-sufficient-entropy";
const AUDIENCE: &str = "/hephaestus.agent.v1.AgentService/ImportAgent";

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    aud: &'a str,
    sub: String,
    jti: String,
    iat: i64,
    nbf: i64,
    exp: i64,
    sid: Option<String>,
}

#[derive(Serialize)]
struct BootstrapClaims<'a> {
    iss: &'a str,
    aud: &'a str,
    sub: &'a str,
    jti: String,
    iat: i64,
    nbf: i64,
    exp: i64,
    actor_kind: &'a str,
    oidc_iss: &'a str,
    oidc_sub: &'a str,
    name: &'a str,
    email: &'a str,
    email_verified: bool,
}

#[derive(Serialize)]
struct MinimalBootstrapClaims<'a> {
    iss: &'a str,
    aud: &'a str,
    sub: &'a str,
    jti: String,
    iat: i64,
    nbf: i64,
    exp: i64,
    actor_kind: &'a str,
    oidc_iss: &'a str,
    oidc_sub: &'a str,
}

#[derive(Clone, Copy)]
enum FakeSessionOutcome {
    Active,
    Unauthenticated,
    Unavailable,
}

#[derive(Clone)]
struct RecordingSessionStore {
    expected_user: UserId,
    expected_sid: BrowserSessionSid,
    metadata: BrowserSessionMetadata,
    outcome: FakeSessionOutcome,
    calls: Arc<Mutex<Vec<(UserId, BrowserSessionSid)>>>,
}

#[derive(Clone, Default)]
struct RecordingAuditSink {
    events: Arc<Mutex<Vec<NewUiRequestAuditEvent>>>,
    fail: bool,
}

#[derive(Clone, Copy, Default)]
struct HangingAuditSink;

#[async_trait]
impl UiRequestAuditSink for HangingAuditSink {
    async fn append(
        &self,
        _event: NewUiRequestAuditEvent,
    ) -> Result<(), release_service::UiRequestAuditError> {
        pending().await
    }
}

#[async_trait]
impl UiRequestAuditSink for RecordingAuditSink {
    async fn append(
        &self,
        event: NewUiRequestAuditEvent,
    ) -> Result<(), release_service::UiRequestAuditError> {
        if self.fail {
            return Err(release_service::UiRequestAuditError::Unavailable);
        }
        self.events.lock().expect("audit sink lock").push(event);
        Ok(())
    }
}

#[async_trait]
impl BrowserSessionStore for RecordingSessionStore {
    async fn create_browser_session(
        &self,
        _command: CreateBrowserSession,
    ) -> Result<CreatedBrowserSession, CreateBrowserSessionError> {
        Err(CreateBrowserSessionError::Unavailable)
    }

    async fn authenticate_browser_session(
        &self,
        user_id: UserId,
        sid: BrowserSessionSid,
    ) -> Result<BrowserSessionMetadata, BrowserSessionAuthenticationError> {
        self.calls
            .lock()
            .expect("recording store lock")
            .push((user_id, sid));
        if user_id != self.expected_user || sid != self.expected_sid {
            return Err(BrowserSessionAuthenticationError::Unauthenticated);
        }
        match self.outcome {
            FakeSessionOutcome::Active => Ok(self.metadata),
            FakeSessionOutcome::Unauthenticated => {
                Err(BrowserSessionAuthenticationError::Unauthenticated)
            }
            FakeSessionOutcome::Unavailable => Err(BrowserSessionAuthenticationError::Unavailable),
        }
    }

    async fn revoke_browser_session(
        &self,
        _command: RevokeBrowserSession,
    ) -> Result<RevokedBrowserSession, RevokeBrowserSessionError> {
        Err(RevokeBrowserSessionError::Unavailable)
    }
}

fn session_metadata(user_id: UserId) -> BrowserSessionMetadata {
    let issued_at = OffsetDateTime::now_utc();
    BrowserSessionMetadata::new(
        BrowserSessionId::new(),
        user_id,
        issued_at,
        issued_at + Duration::hours(1),
        None,
    )
    .expect("valid test session metadata")
}

async fn dispatch_status(
    state: MediatorAuthenticationState,
    path: &str,
    token: Option<&str>,
) -> StatusCode {
    let app = Router::new()
        .fallback(|| async { StatusCode::NO_CONTENT })
        .layer(axum::middleware::from_fn_with_state(
            state,
            super::mediator_identity_middleware,
        ));
    let mut request = Request::builder().uri(path);
    if let Some(token) = token {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    app.oneshot(
        request
            .body(Body::empty())
            .expect("test request should build"),
    )
    .await
    .expect("middleware response")
    .status()
}

async fn inspect_extensions(request: Request<Body>) -> StatusCode {
    let identity = request
        .extensions()
        .get::<identity_domain::AuthenticatedIdentity>()
        .expect("identity extension");
    let session = request
        .extensions()
        .get::<super::VerifiedMediatorSession>()
        .expect("session extension");
    let sid = session.sid.to_protocol_string();
    let debug = format!("{identity:?}{session:?}");
    if debug.contains(&sid) {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::NO_CONTENT
    }
}

mod claims;
mod middleware;
mod session;

fn assertion(
    key: &[u8],
    audience: &str,
    user_id: Uuid,
    assertion_id: Uuid,
    issued_at: i64,
    expires_at: i64,
) -> String {
    assertion_with_sid(
        key,
        audience,
        user_id,
        assertion_id,
        issued_at,
        expires_at,
        BrowserSessionSid::new(),
    )
}

fn assertion_with_sid(
    key: &[u8],
    audience: &str,
    user_id: Uuid,
    assertion_id: Uuid,
    issued_at: i64,
    expires_at: i64,
    sid: BrowserSessionSid,
) -> String {
    encode(
        &Header::new(Algorithm::HS256),
        &Claims {
            iss: ISSUER,
            aud: audience,
            sub: user_id.to_string(),
            jti: assertion_id.to_string(),
            iat: issued_at,
            nbf: issued_at,
            exp: expires_at,
            sid: Some(sid.to_protocol_string()),
        },
        &EncodingKey::from_secret(key),
    )
    .expect("encode assertion")
}

fn assertion_without_sid(
    key: &[u8],
    audience: &str,
    user_id: Uuid,
    assertion_id: Uuid,
    issued_at: i64,
    expires_at: i64,
) -> String {
    encode(
        &Header::new(Algorithm::HS256),
        &Claims {
            iss: ISSUER,
            aud: audience,
            sub: user_id.to_string(),
            jti: assertion_id.to_string(),
            iat: issued_at,
            nbf: issued_at,
            exp: expires_at,
            sid: None,
        },
        &EncodingKey::from_secret(key),
    )
    .expect("encode sidless assertion")
}

fn headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("authorization header"),
    );
    headers
}
