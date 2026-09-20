use axum::{body::Body, extract::State, http::Request, middleware::Next, response::Response};
use http::{HeaderMap, StatusCode, header::AUTHORIZATION};
use identity_application::{BrowserSessionAuthenticationError, BrowserSessionStore};
use identity_domain::{BrowserSessionId, BrowserSessionSid, UserId};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use std::{collections::HashSet, str::FromStr, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;

const ISSUER: &str = "hephaestus-web-mediator";
const BOOTSTRAP_SUBJECT: &str = "hephaestus-web-mediator";
const BOOTSTRAP_ACTOR_KIND: &str = "verified_oidc_bootstrap";
const BOOTSTRAP_AUDIENCE: &str = "/hephaestus.identity.v1.IdentityService/ResolveIdentity";
const CREATE_BOOTSTRAP_AUDIENCE: &str =
    "/hephaestus.identity.v1.IdentityService/CreateBrowserSession";
const REVOKE_AUDIENCE: &str = "/hephaestus.identity.v1.IdentityService/RevokeBrowserSession";
const MAX_LIFETIME_SECONDS: i64 = 30;
const CLOCK_SKEW_SECONDS: i64 = 5;

/// Authenticated mediator subject safe to convert into application identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediatorPrincipal {
    /// Internal user selected by the trusted mediator assertion.
    pub user_id: UserId,
    /// Unique assertion identifier retained only for audit correlation.
    pub assertion_id: Uuid,
    /// Browser SID carried by the signed mediator assertion.
    pub sid: BrowserSessionSid,
}

/// Identity fields a bootstrap assertion must bind to the request body.
pub struct BootstrapIdentity<'a> {
    /// Verified external OIDC issuer.
    pub issuer: &'a str,
    /// Verified external OIDC subject.
    pub subject: &'a str,
    /// Display name received from the verified identity.
    pub display_name: &'a str,
    /// Email received from the verified identity.
    pub email: &'a str,
    /// Whether the upstream issuer verified the email.
    pub email_verified: bool,
}

/// Typed mediator session claims installed by the edge.
///
/// Normal RPCs carry claims that have passed the durable active-session check.
/// Revoke carries signed claims only so an expired session can self-revoke; the
/// revoke handler must not treat that path as proof of current activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedMediatorSession {
    /// Internal user selected by the signed mediator assertion.
    pub user_id: UserId,
    /// Unique assertion identifier retained only for audit correlation.
    pub assertion_id: Uuid,
    /// Browser SID carried by the signed assertion; active routes verify it
    /// against `PostgreSQL` before admitting the request.
    pub sid: BrowserSessionSid,
    /// Internal durable row identity returned by the active-session check.
    ///
    /// Signed-only routes such as revoke intentionally leave this absent;
    /// only active middleware authentication can populate it.
    pub parent_session_id: Option<BrowserSessionId>,
}

/// Cloneable middleware state for signed mediator and browser-session checks.
#[derive(Clone)]
pub struct MediatorAuthenticationState {
    authenticator: MediatorAuthenticator,
    browser_sessions: Arc<dyn BrowserSessionStore>,
}

impl MediatorAuthenticationState {
    /// Creates middleware state with the application-role session verifier.
    #[must_use]
    pub fn new(
        authenticator: MediatorAuthenticator,
        browser_sessions: Arc<dyn BrowserSessionStore>,
    ) -> Self {
        Self {
            authenticator,
            browser_sessions,
        }
    }

    fn authenticate_signed(
        &self,
        headers: &HeaderMap,
        expected_audience: &str,
    ) -> Result<VerifiedMediatorSession, MediatorAssertionError> {
        let principal = self
            .authenticator
            .authenticate(headers, expected_audience)?;
        Ok(VerifiedMediatorSession {
            user_id: principal.user_id,
            assertion_id: principal.assertion_id,
            sid: principal.sid,
            parent_session_id: None,
        })
    }

    async fn authenticate_active(
        &self,
        headers: &HeaderMap,
        expected_audience: &str,
    ) -> Result<VerifiedMediatorSession, SessionAuthenticationError> {
        let session = self
            .authenticate_signed(headers, expected_audience)
            .map_err(|_| SessionAuthenticationError::Unauthenticated)?;
        let metadata = self
            .browser_sessions
            .authenticate_browser_session(session.user_id, session.sid)
            .await
            .map_err(|error| match error {
                BrowserSessionAuthenticationError::Unauthenticated => {
                    SessionAuthenticationError::Unauthenticated
                }
                BrowserSessionAuthenticationError::Unavailable => {
                    SessionAuthenticationError::Unavailable
                }
            })?;
        if metadata.user_id() != session.user_id {
            return Err(SessionAuthenticationError::Unauthenticated);
        }
        Ok(VerifiedMediatorSession {
            parent_session_id: Some(metadata.id()),
            ..session
        })
    }
}

/// Verifies audience-bound, short-lived Phoenix mediator assertions.
#[derive(Clone)]
pub struct MediatorAuthenticator {
    decoding_key: DecodingKey,
}

impl MediatorAuthenticator {
    /// Creates an authenticator from the domain-separated HS256 key.
    #[must_use]
    pub fn new(signing_key: &[u8]) -> Self {
        Self {
            decoding_key: DecodingKey::from_secret(signing_key),
        }
    }

    /// Authenticates the bearer assertion for one exact RPC procedure.
    ///
    /// # Errors
    ///
    /// Returns one non-sensitive error for missing, malformed, expired,
    /// overlong, or wrong-audience assertions.
    pub fn authenticate(
        &self,
        headers: &HeaderMap,
        expected_audience: &str,
    ) -> Result<MediatorPrincipal, MediatorAssertionError> {
        let token = bearer_token(headers)?;
        let validation = mediator_validation(expected_audience);
        let claims = decode::<MediatorClaims>(token, &self.decoding_key, &validation)
            .map_err(|_| MediatorAssertionError)?
            .claims;
        claims.validate_times()?;
        Ok(MediatorPrincipal {
            user_id: UserId::from_str(&claims.sub).map_err(|_| MediatorAssertionError)?,
            assertion_id: Uuid::parse_str(&claims.jti).map_err(|_| MediatorAssertionError)?,
            sid: BrowserSessionSid::from_str(claims.sid.as_deref().ok_or(MediatorAssertionError)?)
                .map_err(|_| MediatorAssertionError)?,
        })
    }

    /// Authenticates one bootstrap assertion by its verified OIDC binding.
    ///
    /// This deliberately checks only issuer and subject. The full `ResolveIdentity`
    /// bootstrap retains its display and email binding in `authenticate_bootstrap`.
    ///
    /// # Errors
    ///
    /// Returns one non-sensitive error unless the signed bootstrap assertion
    /// has the expected audience, actor kind, issuer, and subject.
    pub fn authenticate_session_bootstrap(
        &self,
        headers: &HeaderMap,
        expected_audience: &str,
        issuer: &str,
        subject: &str,
    ) -> Result<Uuid, MediatorAssertionError> {
        let token = bearer_token(headers)?;
        let mut validation = mediator_validation(expected_audience);
        validation.sub = Some(String::from(BOOTSTRAP_SUBJECT));
        let claims = decode::<SessionBootstrapClaims>(token, &self.decoding_key, &validation)
            .map_err(|_| MediatorAssertionError)?
            .claims;
        claims.registered.validate_times()?;
        if claims.actor_kind != BOOTSTRAP_ACTOR_KIND
            || claims.oidc_iss != issuer
            || claims.oidc_sub != subject
        {
            return Err(MediatorAssertionError);
        }
        Uuid::parse_str(&claims.registered.jti).map_err(|_| MediatorAssertionError)
    }

    /// Authenticates the method-specific identity-resolution bootstrap.
    ///
    /// # Errors
    ///
    /// Returns one non-sensitive error unless every signed identity field
    /// exactly matches the request supplied by the web mediator.
    pub fn authenticate_bootstrap(
        &self,
        headers: &HeaderMap,
        expected_audience: &str,
        expected: &BootstrapIdentity<'_>,
    ) -> Result<Uuid, MediatorAssertionError> {
        let token = bearer_token(headers)?;
        let mut validation = mediator_validation(expected_audience);
        validation.sub = Some(String::from(BOOTSTRAP_SUBJECT));
        let claims = decode::<BootstrapClaims>(token, &self.decoding_key, &validation)
            .map_err(|_| MediatorAssertionError)?
            .claims;
        claims.registered.validate_times()?;
        if claims.actor_kind != BOOTSTRAP_ACTOR_KIND
            || claims.oidc_iss != expected.issuer
            || claims.oidc_sub != expected.subject
            || claims.name != expected.display_name
            || claims.email != expected.email
            || claims.email_verified != expected.email_verified
        {
            return Err(MediatorAssertionError);
        }
        Uuid::parse_str(&claims.registered.jti).map_err(|_| MediatorAssertionError)
    }
}

/// Axum middleware that validates a mediator assertion for the exact request
/// path and installs the resulting identity in request extensions.
///
/// The two bootstrap procedures validate their request-body identity later in
/// their typed RPC handlers. Revoke authenticates the signed session claims but
/// deliberately skips the active-row check so an expired session can log out.
/// Every other Connect RPC requires an active durable browser session.
pub async fn mediator_identity_middleware(
    State(state): State<MediatorAuthenticationState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let mode = auth_mode(request.uri().path());
    let session = match mode {
        MediatorAuthMode::Public | MediatorAuthMode::Bootstrap => {
            return next.run(request).await;
        }
        MediatorAuthMode::Signed => {
            match state.authenticate_signed(request.headers(), request.uri().path()) {
                Ok(session) => session,
                Err(_) => return auth_response(StatusCode::UNAUTHORIZED),
            }
        }
        MediatorAuthMode::Active => match state
            .authenticate_active(request.headers(), request.uri().path())
            .await
        {
            Ok(session) => session,
            Err(SessionAuthenticationError::Unauthenticated) => {
                return auth_response(StatusCode::UNAUTHORIZED);
            }
            Err(SessionAuthenticationError::Unavailable) => {
                return auth_response(StatusCode::SERVICE_UNAVAILABLE);
            }
        },
    };
    request.extensions_mut().insert(session);
    request
        .extensions_mut()
        .insert(identity_domain::AuthenticatedIdentity::new(
            session.user_id,
            ISSUER,
            session.user_id.to_string(),
            serde_json::json!({"mediator": "phoenix", "assertion_id": session.assertion_id}),
            identity_domain::RequestId::from_uuid(session.assertion_id),
        ));
    next.run(request).await
}

fn auth_response(status: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionAuthenticationError {
    Unauthenticated,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediatorAuthMode {
    Public,
    Bootstrap,
    Signed,
    Active,
}

fn auth_mode(path: &str) -> MediatorAuthMode {
    if !path.starts_with("/hephaestus.") {
        return MediatorAuthMode::Public;
    }
    match path {
        BOOTSTRAP_AUDIENCE | CREATE_BOOTSTRAP_AUDIENCE => MediatorAuthMode::Bootstrap,
        REVOKE_AUDIENCE => MediatorAuthMode::Signed,
        _ => MediatorAuthMode::Active,
    }
}

#[cfg(test)]
fn requires_mediator_auth(path: &str) -> bool {
    !matches!(
        auth_mode(path),
        MediatorAuthMode::Public | MediatorAuthMode::Bootstrap
    )
}

#[derive(Deserialize)]
struct MediatorClaims {
    sub: String,
    jti: String,
    #[serde(default)]
    sid: Option<String>,
    iat: i64,
    nbf: i64,
    exp: i64,
}

#[derive(Deserialize)]
struct BootstrapClaims {
    #[serde(flatten)]
    registered: MediatorClaims,
    actor_kind: String,
    oidc_iss: String,
    oidc_sub: String,
    name: String,
    email: String,
    email_verified: bool,
}

#[derive(Deserialize)]
struct SessionBootstrapClaims {
    #[serde(flatten)]
    registered: MediatorClaims,
    actor_kind: String,
    oidc_iss: String,
    oidc_sub: String,
}

impl MediatorClaims {
    fn validate_times(&self) -> Result<(), MediatorAssertionError> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let lifetime = self
            .exp
            .checked_sub(self.iat)
            .ok_or(MediatorAssertionError)?;
        if !(0..=MAX_LIFETIME_SECONDS).contains(&lifetime)
            || self.nbf < self.iat
            || self.nbf > self.exp
            || self.iat > now + CLOCK_SKEW_SECONDS
            || self.exp < now - CLOCK_SKEW_SECONDS
        {
            return Err(MediatorAssertionError);
        }
        Ok(())
    }
}

/// Non-sensitive mediator authentication failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("mediator authentication failed")]
pub struct MediatorAssertionError;

fn bearer_token(headers: &HeaderMap) -> Result<&str, MediatorAssertionError> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(MediatorAssertionError)
}

fn mediator_validation(expected_audience: &str) -> Validation {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[expected_audience]);
    validation.leeway = u64::try_from(CLOCK_SKEW_SECONDS).expect("positive clock skew");
    validation.required_spec_claims = HashSet::from([
        String::from("aud"),
        String::from("exp"),
        String::from("iat"),
        String::from("iss"),
        String::from("jti"),
        String::from("nbf"),
        String::from("sub"),
    ]);
    validation
}

#[cfg(test)]
mod tests {
    use super::{
        BOOTSTRAP_ACTOR_KIND, BOOTSTRAP_AUDIENCE, BOOTSTRAP_SUBJECT, BootstrapIdentity,
        CLOCK_SKEW_SECONDS, CREATE_BOOTSTRAP_AUDIENCE, ISSUER, MediatorAuthenticationState,
        MediatorAuthenticator, REVOKE_AUDIENCE, requires_mediator_auth,
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
    use serde::Serialize;
    use std::sync::{Arc, Mutex};
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
                FakeSessionOutcome::Unavailable => {
                    Err(BrowserSessionAuthenticationError::Unavailable)
                }
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

    #[tokio::test]
    async fn active_middleware_checks_sid_and_maps_store_results() {
        let key = mediator_signing_key(TOKEN);
        let user_id = UserId::new();
        let sid = BrowserSessionSid::new();
        let assertion_id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let token = assertion_with_sid(
            &key,
            AUDIENCE,
            user_id.as_uuid(),
            assertion_id,
            now,
            now + 30,
            sid,
        );
        let calls = Arc::new(Mutex::new(Vec::new()));
        let state = MediatorAuthenticationState::new(
            MediatorAuthenticator::new(&key),
            Arc::new(RecordingSessionStore {
                expected_user: user_id,
                expected_sid: sid,
                metadata: session_metadata(user_id),
                outcome: FakeSessionOutcome::Active,
                calls: Arc::clone(&calls),
            }),
        );
        assert_eq!(
            dispatch_status(state, AUDIENCE, Some(&token)).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            *calls.lock().expect("recording store lock"),
            vec![(user_id, sid)]
        );

        let unauthenticated = MediatorAuthenticationState::new(
            MediatorAuthenticator::new(&key),
            Arc::new(RecordingSessionStore {
                expected_user: user_id,
                expected_sid: sid,
                metadata: session_metadata(user_id),
                outcome: FakeSessionOutcome::Unauthenticated,
                calls: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        assert_eq!(
            dispatch_status(unauthenticated, AUDIENCE, Some(&token)).await,
            StatusCode::UNAUTHORIZED
        );

        let unavailable = MediatorAuthenticationState::new(
            MediatorAuthenticator::new(&key),
            Arc::new(RecordingSessionStore {
                expected_user: user_id,
                expected_sid: sid,
                metadata: session_metadata(user_id),
                outcome: FakeSessionOutcome::Unavailable,
                calls: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        assert_eq!(
            dispatch_status(unavailable, AUDIENCE, Some(&token)).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn active_authentication_retains_exact_store_session_id() {
        let key = mediator_signing_key(TOKEN);
        let user_id = UserId::new();
        let sid = BrowserSessionSid::new();
        let metadata = session_metadata(user_id);
        let expected_session_id = metadata.id();
        let state = MediatorAuthenticationState::new(
            MediatorAuthenticator::new(&key),
            Arc::new(RecordingSessionStore {
                expected_user: user_id,
                expected_sid: sid,
                metadata,
                outcome: FakeSessionOutcome::Active,
                calls: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let token = assertion_with_sid(
            &key,
            AUDIENCE,
            user_id.as_uuid(),
            Uuid::new_v4(),
            now,
            now + 30,
            sid,
        );

        let authenticated = state
            .authenticate_active(&headers(&token), AUDIENCE)
            .await
            .expect("active session should authenticate");
        assert_eq!(authenticated.parent_session_id, Some(expected_session_id));
    }

    #[tokio::test]
    async fn revoke_skips_active_lookup_but_keeps_signed_audience_and_sid_checks() {
        let key = mediator_signing_key(TOKEN);
        let user_id = UserId::new();
        let sid = BrowserSessionSid::new();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let state = MediatorAuthenticationState::new(
            MediatorAuthenticator::new(&key),
            Arc::new(RecordingSessionStore {
                expected_user: user_id,
                expected_sid: sid,
                metadata: session_metadata(user_id),
                outcome: FakeSessionOutcome::Unavailable,
                calls: Arc::clone(&calls),
            }),
        );
        let token = assertion_with_sid(
            &key,
            REVOKE_AUDIENCE,
            user_id.as_uuid(),
            Uuid::new_v4(),
            now,
            now + 30,
            sid,
        );
        let signed = state
            .authenticate_signed(&headers(&token), REVOKE_AUDIENCE)
            .expect("signed revoke assertion should authenticate");
        assert_eq!(signed.parent_session_id, None);
        assert_eq!(
            dispatch_status(state, REVOKE_AUDIENCE, Some(&token)).await,
            StatusCode::NO_CONTENT
        );
        assert!(calls.lock().expect("recording store lock").is_empty());

        let wrong_audience = assertion_with_sid(
            &key,
            AUDIENCE,
            user_id.as_uuid(),
            Uuid::new_v4(),
            now,
            now + 30,
            sid,
        );
        let state = MediatorAuthenticationState::new(
            MediatorAuthenticator::new(&key),
            Arc::new(RecordingSessionStore {
                expected_user: user_id,
                expected_sid: sid,
                metadata: session_metadata(user_id),
                outcome: FakeSessionOutcome::Active,
                calls: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        assert_eq!(
            dispatch_status(state, REVOKE_AUDIENCE, Some(&wrong_audience)).await,
            StatusCode::UNAUTHORIZED
        );
        let sidless = assertion_without_sid(
            &key,
            REVOKE_AUDIENCE,
            user_id.as_uuid(),
            Uuid::new_v4(),
            now,
            now + 30,
        );
        let state = MediatorAuthenticationState::new(
            MediatorAuthenticator::new(&key),
            Arc::new(RecordingSessionStore {
                expected_user: user_id,
                expected_sid: sid,
                metadata: session_metadata(user_id),
                outcome: FakeSessionOutcome::Active,
                calls: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        assert_eq!(
            dispatch_status(state, REVOKE_AUDIENCE, Some(&sidless)).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn only_exact_bootstrap_paths_skip_the_session_gate_and_debug_redacts_sid() {
        let key = mediator_signing_key(TOKEN);
        let user_id = UserId::new();
        let sid = BrowserSessionSid::new();
        let store = Arc::new(RecordingSessionStore {
            expected_user: user_id,
            expected_sid: sid,
            metadata: session_metadata(user_id),
            outcome: FakeSessionOutcome::Unavailable,
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let state = MediatorAuthenticationState::new(
            MediatorAuthenticator::new(&key),
            Arc::clone(&store) as Arc<dyn BrowserSessionStore>,
        );
        assert_eq!(
            dispatch_status(state, BOOTSTRAP_AUDIENCE, None).await,
            StatusCode::NO_CONTENT
        );
        assert!(store.calls.lock().expect("recording store lock").is_empty());

        let nearby = "/hephaestus.identity.v1.IdentityService/ResolveIdentity/extra";
        let token = assertion_with_sid(
            &key,
            nearby,
            user_id.as_uuid(),
            Uuid::new_v4(),
            OffsetDateTime::now_utc().unix_timestamp(),
            OffsetDateTime::now_utc().unix_timestamp() + 30,
            sid,
        );
        let nearby_state = MediatorAuthenticationState::new(
            MediatorAuthenticator::new(&key),
            Arc::clone(&store) as Arc<dyn BrowserSessionStore>,
        );
        assert_eq!(
            dispatch_status(nearby_state, nearby, Some(&token)).await,
            StatusCode::SERVICE_UNAVAILABLE
        );

        let active_calls = Arc::new(Mutex::new(Vec::new()));
        let active_store = Arc::new(RecordingSessionStore {
            expected_user: user_id,
            expected_sid: sid,
            metadata: session_metadata(user_id),
            outcome: FakeSessionOutcome::Active,
            calls: active_calls,
        });
        let active_state = MediatorAuthenticationState::new(
            MediatorAuthenticator::new(&key),
            Arc::clone(&active_store) as Arc<dyn BrowserSessionStore>,
        );
        let app =
            Router::new()
                .fallback(inspect_extensions)
                .layer(axum::middleware::from_fn_with_state(
                    active_state,
                    super::mediator_identity_middleware,
                ));
        let token = assertion_with_sid(
            &key,
            AUDIENCE,
            user_id.as_uuid(),
            Uuid::new_v4(),
            OffsetDateTime::now_utc().unix_timestamp(),
            OffsetDateTime::now_utc().unix_timestamp() + 30,
            sid,
        );
        let mut request = Request::builder().uri(AUDIENCE);
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        assert_eq!(
            app.oneshot(request.body(Body::empty()).expect("test request"))
                .await
                .expect("middleware response")
                .status(),
            StatusCode::NO_CONTENT
        );
    }

    #[test]
    fn accepts_only_exact_short_lived_audience_bound_assertions() {
        let key = mediator_signing_key(TOKEN);
        let authenticator = MediatorAuthenticator::new(&key);
        let user_id = Uuid::new_v4();
        let assertion_id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let token = assertion(&key, AUDIENCE, user_id, assertion_id, now, now + 30);
        let valid_headers = headers(&token);

        let principal = authenticator
            .authenticate(&valid_headers, AUDIENCE)
            .expect("valid assertion");
        assert_eq!(principal.user_id.to_string(), user_id.to_string());
        assert_eq!(principal.assertion_id, assertion_id);
        assert!(principal.sid.to_protocol_string().parse::<Uuid>().is_ok());
        assert!(
            authenticator
                .authenticate(&valid_headers, "/wrong.Service/Method")
                .is_err()
        );

        let overlong = assertion(&key, AUDIENCE, user_id, assertion_id, now, now + 31);
        assert!(
            authenticator
                .authenticate(&headers(&overlong), AUDIENCE)
                .is_err()
        );

        let future = assertion(
            &key,
            AUDIENCE,
            user_id,
            assertion_id,
            now + CLOCK_SKEW_SECONDS + 1,
            now + CLOCK_SKEW_SECONDS + 2,
        );
        assert!(
            authenticator
                .authenticate(&headers(&future), AUDIENCE)
                .is_err()
        );
    }

    #[test]
    fn errors_never_contain_assertion_material() {
        let authenticator = MediatorAuthenticator::new(&mediator_signing_key(TOKEN));
        let sentinel = "sensitive-assertion-sentinel";
        let error = authenticator
            .authenticate(&headers(sentinel), AUDIENCE)
            .expect_err("malformed assertion must fail");
        assert!(!error.to_string().contains(sentinel));
        assert!(!format!("{error:?}").contains(sentinel));
    }

    #[test]
    fn normal_assertions_require_a_sid_without_leaking_token_material() {
        let key = mediator_signing_key(TOKEN);
        let authenticator = MediatorAuthenticator::new(&key);
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let token = encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                iss: ISSUER,
                aud: AUDIENCE,
                sub: Uuid::new_v4().to_string(),
                jti: Uuid::new_v4().to_string(),
                iat: now,
                nbf: now,
                exp: now + 30,
                sid: None,
            },
            &EncodingKey::from_secret(&key),
        )
        .expect("encode sidless assertion");
        let error = authenticator
            .authenticate(&headers(&token), AUDIENCE)
            .expect_err("normal assertion without sid must fail");
        assert!(!error.to_string().contains(&token));
        assert!(!format!("{error:?}").contains(&token));
    }

    #[test]
    fn bootstrap_and_revoke_are_the_only_signed_session_exceptions() {
        assert!(!requires_mediator_auth(BOOTSTRAP_AUDIENCE));
        assert!(!requires_mediator_auth(CREATE_BOOTSTRAP_AUDIENCE));
        assert!(requires_mediator_auth(REVOKE_AUDIENCE));
        assert!(requires_mediator_auth(
            "/hephaestus.identity.v1.IdentityService/ResolveIdentity/extra"
        ));
    }

    #[test]
    fn middleware_authenticates_only_connect_paths() {
        assert!(requires_mediator_auth(
            "/hephaestus.agent.v1.AgentService/ImportAgent"
        ));
        assert!(!requires_mediator_auth(BOOTSTRAP_AUDIENCE));
        assert!(!requires_mediator_auth("/healthz"));
        assert!(!requires_mediator_auth("/git/repository/info/refs"));
        assert!(!requires_mediator_auth("/"));
    }

    #[test]
    fn every_service_domain_requires_its_exact_audience() {
        let audiences = [
            "/hephaestus.artifact.v1.ArtifactService/GetArtifactPreview",
            "/hephaestus.build.v1.BuildService/GetBuild",
            "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
            "/hephaestus.organization.v1.OrganizationService/ListOrganizations",
            "/hephaestus.project.v1.ProjectService/GetProject",
            "/hephaestus.release.v1.ReleaseService/GetRelease",
            "/hephaestus.repository.v1.RepositoryService/GetRepository",
            "/hephaestus.repository_browser.v1.RepositoryBrowserService/ListBranches",
            "/hephaestus.run.v1.RunService/GetRun",
            "/hephaestus.secret.v1.SecretService/ListProjectSecrets",
        ];
        let key = mediator_signing_key(TOKEN);
        let authenticator = MediatorAuthenticator::new(&key);
        let now = OffsetDateTime::now_utc().unix_timestamp();
        for audience in audiences {
            let token = assertion(
                &key,
                audience,
                Uuid::new_v4(),
                Uuid::new_v4(),
                now,
                now + 30,
            );
            let headers = headers(&token);
            assert!(authenticator.authenticate(&headers, audience).is_ok());
            assert!(
                authenticator
                    .authenticate(&headers, "/wrong.Service/Method")
                    .is_err()
            );
        }
    }

    #[test]
    fn bootstrap_assertion_binds_every_verified_identity_field() {
        let key = mediator_signing_key(TOKEN);
        let authenticator = MediatorAuthenticator::new(&key);
        let assertion_id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let audience = "/hephaestus.identity.v1.IdentityService/ResolveIdentity";
        let expected = BootstrapIdentity {
            issuer: "https://issuer.example",
            subject: "external-subject",
            display_name: "Ada",
            email: "ada@example.test",
            email_verified: true,
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &BootstrapClaims {
                iss: ISSUER,
                aud: audience,
                sub: BOOTSTRAP_SUBJECT,
                jti: assertion_id.to_string(),
                iat: now,
                nbf: now,
                exp: now + 30,
                actor_kind: BOOTSTRAP_ACTOR_KIND,
                oidc_iss: expected.issuer,
                oidc_sub: expected.subject,
                name: expected.display_name,
                email: expected.email,
                email_verified: expected.email_verified,
            },
            &EncodingKey::from_secret(&key),
        )
        .expect("encode bootstrap assertion");
        assert_eq!(
            authenticator
                .authenticate_bootstrap(&headers(&token), audience, &expected)
                .expect("valid bootstrap"),
            assertion_id
        );

        let altered = BootstrapIdentity {
            email: "attacker@example.test",
            ..expected
        };
        assert!(
            authenticator
                .authenticate_bootstrap(&headers(&token), audience, &altered)
                .is_err()
        );
    }

    #[test]
    fn session_bootstrap_binds_only_verified_oidc_identity() {
        let key = mediator_signing_key(TOKEN);
        let authenticator = MediatorAuthenticator::new(&key);
        let assertion_id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let token = encode(
            &Header::new(Algorithm::HS256),
            &MinimalBootstrapClaims {
                iss: ISSUER,
                aud: CREATE_BOOTSTRAP_AUDIENCE,
                sub: BOOTSTRAP_SUBJECT,
                jti: assertion_id.to_string(),
                iat: now,
                nbf: now,
                exp: now + 30,
                actor_kind: BOOTSTRAP_ACTOR_KIND,
                oidc_iss: "https://issuer.example",
                oidc_sub: "external-subject",
            },
            &EncodingKey::from_secret(&key),
        )
        .expect("encode session bootstrap assertion");
        let headers = headers(&token);
        assert_eq!(
            authenticator
                .authenticate_session_bootstrap(
                    &headers,
                    CREATE_BOOTSTRAP_AUDIENCE,
                    "https://issuer.example",
                    "external-subject",
                )
                .expect("valid session bootstrap"),
            assertion_id
        );
        assert!(
            authenticator
                .authenticate_session_bootstrap(
                    &headers,
                    CREATE_BOOTSTRAP_AUDIENCE,
                    "https://attacker.example",
                    "external-subject",
                )
                .is_err()
        );
        assert!(
            authenticator
                .authenticate_session_bootstrap(
                    &headers,
                    CREATE_BOOTSTRAP_AUDIENCE,
                    "https://issuer.example",
                    "other-subject",
                )
                .is_err()
        );
    }

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
}
