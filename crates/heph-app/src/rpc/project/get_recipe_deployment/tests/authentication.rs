//! Exercises actual middleware on the new method path with a controlled session port.

use super::super::AUDIENCE;
use crate::rpc::{
    MediatorAuthenticationState, MediatorAuthenticator, mediator_identity_middleware,
    mediator_signing_key,
};
use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use identity_application::{
    BrowserSessionAuthenticationError, BrowserSessionStore, CreateBrowserSession,
    CreateBrowserSessionError, CreatedBrowserSession, RevokeBrowserSession,
    RevokeBrowserSessionError, RevokedBrowserSession,
};
use identity_domain::{
    AuthenticatedIdentity, BrowserSessionId, BrowserSessionMetadata, BrowserSessionSid, UserId,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use time::{Duration, OffsetDateTime};
use tower::ServiceExt;
use uuid::Uuid;

struct Sessions {
    metadata: BrowserSessionMetadata,
    active: bool,
    expected_sid: BrowserSessionSid,
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl BrowserSessionStore for Sessions {
    async fn create_browser_session(
        &self,
        _: CreateBrowserSession,
    ) -> Result<CreatedBrowserSession, CreateBrowserSessionError> {
        Err(CreateBrowserSessionError::Unavailable)
    }
    async fn authenticate_browser_session(
        &self,
        user: UserId,
        sid: BrowserSessionSid,
    ) -> Result<BrowserSessionMetadata, BrowserSessionAuthenticationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.active && user == self.metadata.user_id() && sid == self.expected_sid {
            Ok(self.metadata)
        } else {
            Err(BrowserSessionAuthenticationError::Unauthenticated)
        }
    }
    async fn revoke_browser_session(
        &self,
        _: RevokeBrowserSession,
    ) -> Result<RevokedBrowserSession, RevokeBrowserSessionError> {
        Err(RevokeBrowserSessionError::Unavailable)
    }
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    aud: &'a str,
    sub: String,
    jti: String,
    iat: i64,
    nbf: i64,
    exp: i64,
    sid: String,
}

async fn dispatch(
    token: Option<&str>,
    active: bool,
    user: UserId,
    expected_sid: BrowserSessionSid,
    calls: Arc<AtomicUsize>,
) -> StatusCode {
    let now = OffsetDateTime::now_utc();
    let sessions = Sessions {
        metadata: BrowserSessionMetadata::new(
            BrowserSessionId::new(),
            user,
            now,
            now + Duration::hours(1),
            None,
        )
        .unwrap(),
        active,
        expected_sid,
        calls,
    };
    let state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&mediator_signing_key(
            b"recipe-inspection-controlled-session-key",
        )),
        Arc::new(sessions),
    );
    let app = Router::new()
        .route(
            AUDIENCE,
            axum::routing::post(|request: Request<Body>| async move {
                if request
                    .extensions()
                    .get::<AuthenticatedIdentity>()
                    .is_some()
                {
                    StatusCode::NO_CONTENT
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }),
        )
        .layer(axum::middleware::from_fn_with_state(
            state,
            mediator_identity_middleware,
        ));
    let mut request = Request::builder().method("POST").uri(AUDIENCE);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    app.oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

fn token(user: UserId, audience: &str, expired: bool, sid: BrowserSessionSid) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let issued = if expired { now - 100 } else { now };
    encode(
        &Header::new(Algorithm::HS256),
        &Claims {
            iss: "hephaestus-web-mediator",
            aud: audience,
            sub: user.to_string(),
            jti: Uuid::new_v4().to_string(),
            iat: issued,
            nbf: issued,
            exp: issued + 30,
            sid: sid.to_protocol_string(),
        },
        &EncodingKey::from_secret(&mediator_signing_key(
            b"recipe-inspection-controlled-session-key",
        )),
    )
    .unwrap()
}

#[tokio::test]
async fn new_query_requires_exact_audience_and_active_session_before_handler() {
    let user = UserId::new();
    let sid = BrowserSessionSid::new();
    let calls = Arc::new(AtomicUsize::new(0));
    for value in [
        None,
        Some(token(user, AUDIENCE, true, sid)),
        Some(token(
            user,
            "/hephaestus.project.v1.ProjectService/GetProject",
            false,
            sid,
        )),
    ] {
        assert_eq!(
            dispatch(value.as_deref(), true, user, sid, Arc::clone(&calls)).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let value = token(user, AUDIENCE, false, sid);
    assert_eq!(
        dispatch(Some(&value), false, user, sid, Arc::clone(&calls)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        dispatch(Some(&value), true, user, sid, Arc::clone(&calls)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
