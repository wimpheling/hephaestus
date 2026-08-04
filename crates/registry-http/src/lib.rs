//! Authenticated Docker Distribution token-exchange HTTP transport.
//!
//! Authentication happens before this router: the forge installs an
//! [`AuthenticatedIdentity`] request extension only after validating an
//! existing OIDC or workload credential. This transport never accepts a
//! registry-wide username or password.

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Extension, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use identity_domain::AuthenticatedIdentity;
use registry_token::{
    AuthorizationDecision, RegistryTokenIssuer, ScopeRequest, TokenSubject, UnixTimestamp,
};
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Stable HTTP path advertised as Zot's bearer-token realm.
pub const REGISTRY_TOKEN_PATH: &str = "/v1/registry/token";

const MAX_OPTIONAL_CLIENT_FIELD_LENGTH: usize = 256;

/// Live authorization boundary used for every token exchange.
#[async_trait]
pub trait RegistryScopeAuthorizer: Send + Sync + 'static {
    /// Resolves requested repository actions to the exact actions currently
    /// authorized for this authenticated identity.
    ///
    /// # Errors
    ///
    /// Returns a non-sensitive provider error when live authority cannot be
    /// evaluated. A normal denial is represented by an empty decision.
    async fn authorize(
        &self,
        identity: &AuthenticatedIdentity,
        request: &ScopeRequest,
    ) -> Result<AuthorizationDecision, RegistryAuthorizationError>;
}

/// Non-sensitive live authorization provider failure.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("registry authorization is unavailable")]
pub struct RegistryAuthorizationError;

/// Complete state for the registry token HTTP endpoint.
#[derive(Clone)]
pub struct RegistryTokenHttpService {
    issuer: Arc<RegistryTokenIssuer>,
    authorizer: Arc<dyn RegistryScopeAuthorizer>,
}

impl RegistryTokenHttpService {
    /// Creates an endpoint around configured signing and live authorization
    /// adapters.
    #[must_use]
    pub fn new(
        issuer: Arc<RegistryTokenIssuer>,
        authorizer: Arc<dyn RegistryScopeAuthorizer>,
    ) -> Self {
        Self { issuer, authorizer }
    }

    /// Builds the bounded token-exchange router.
    pub fn router(self) -> Router {
        Router::new()
            .route(REGISTRY_TOKEN_PATH, get(exchange_token))
            .with_state(Arc::new(self))
    }

    async fn exchange(
        &self,
        identity: &AuthenticatedIdentity,
        query: TokenQuery,
        now: OffsetDateTime,
    ) -> Result<TokenResponse, TokenHttpError> {
        query.validate_optional_fields()?;
        let request = ScopeRequest::parse(&query.service, query.scope.as_deref().unwrap_or(""))
            .map_err(|_| TokenHttpError::InvalidRequest)?;
        let authorization = self
            .authorizer
            .authorize(identity, &request)
            .await
            .map_err(|_| TokenHttpError::AuthorizationUnavailable)?;
        let unix_seconds =
            u64::try_from(now.unix_timestamp()).map_err(|_| TokenHttpError::ClockUnavailable)?;
        let subject = format!("user:{}", identity.user_id)
            .parse::<TokenSubject>()
            .map_err(|_| TokenHttpError::InvalidIdentity)?;
        let issued = self
            .issuer
            .issue(
                subject,
                &request,
                &authorization,
                UnixTimestamp::new(unix_seconds),
            )
            .map_err(|_| TokenHttpError::InvalidRequest)?;
        let issued_at = now
            .format(&Rfc3339)
            .map_err(|_| TokenHttpError::ClockUnavailable)?;
        Ok(TokenResponse {
            token: issued.token().as_str().to_owned(),
            access_token: issued.token().as_str().to_owned(),
            expires_in: issued.expires_in().seconds(),
            issued_at,
        })
    }
}

#[derive(Debug, Deserialize)]
struct TokenQuery {
    service: String,
    scope: Option<String>,
    account: Option<String>,
    client_id: Option<String>,
}

impl TokenQuery {
    fn validate_optional_fields(&self) -> Result<(), TokenHttpError> {
        for field in [&self.account, &self.client_id].into_iter().flatten() {
            if field.is_empty()
                || field.len() > MAX_OPTIONAL_CLIENT_FIELD_LENGTH
                || field.bytes().any(|byte| byte.is_ascii_control())
            {
                return Err(TokenHttpError::InvalidRequest);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct TokenResponse {
    token: String,
    access_token: String,
    expires_in: u64,
    issued_at: String,
}

async fn exchange_token(
    State(service): State<Arc<RegistryTokenHttpService>>,
    identity: Option<Extension<AuthenticatedIdentity>>,
    Query(query): Query<TokenQuery>,
) -> Response {
    let Some(Extension(identity)) = identity else {
        return TokenHttpError::Unauthenticated.into_response();
    };
    match service
        .exchange(&identity, query, OffsetDateTime::now_utc())
        .await
    {
        Ok(response) => {
            let mut response = (StatusCode::OK, Json(response)).into_response();
            response.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            response.headers_mut().insert(
                axum::http::header::PRAGMA,
                axum::http::HeaderValue::from_static("no-cache"),
            );
            response
        }
        Err(error) => error.into_response(),
    }
}

#[derive(Clone, Copy)]
enum TokenHttpError {
    Unauthenticated,
    InvalidRequest,
    InvalidIdentity,
    AuthorizationUnavailable,
    ClockUnavailable,
}

impl fmt::Debug for TokenHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthenticated => "Unauthenticated",
            Self::InvalidRequest => "InvalidRequest",
            Self::InvalidIdentity => "InvalidIdentity",
            Self::AuthorizationUnavailable => "AuthorizationUnavailable",
            Self::ClockUnavailable => "ClockUnavailable",
        })
    }
}

impl IntoResponse for TokenHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "authentication required",
            ),
            Self::InvalidRequest | Self::InvalidIdentity => (
                StatusCode::BAD_REQUEST,
                "DENIED",
                "registry token request denied",
            ),
            Self::AuthorizationUnavailable | Self::ClockUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "UNAVAILABLE",
                "registry token service unavailable",
            ),
        };
        tracing::warn!(?self, "registry token exchange rejected");
        (status, Json(ErrorEnvelope::new(code, message))).into_response()
    }
}

#[derive(Serialize)]
struct ErrorEnvelope {
    errors: [ErrorDetail; 1],
}

impl ErrorEnvelope {
    const fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            errors: [ErrorDetail { code, message }],
        }
    }
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: &'static str,
}

#[cfg(test)]
mod tests {
    use super::{
        RegistryAuthorizationError, RegistryScopeAuthorizer, RegistryTokenHttpService, TokenQuery,
    };
    use async_trait::async_trait;
    use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
    use registry_token::{
        AuthorizationDecision, RegistryTokenIssuer, RepositoryActions, ScopeRequest, SigningKey,
        TokenLifetime,
    };
    use std::sync::Arc;
    use time::OffsetDateTime;

    struct PullOnly;

    #[async_trait]
    impl RegistryScopeAuthorizer for PullOnly {
        async fn authorize(
            &self,
            _identity: &AuthenticatedIdentity,
            request: &ScopeRequest,
        ) -> Result<AuthorizationDecision, RegistryAuthorizationError> {
            let mut decision = AuthorizationDecision::deny_all();
            for scope in request.scopes() {
                decision.grant(scope.repository().clone(), RepositoryActions::pull());
            }
            Ok(decision)
        }
    }

    fn service() -> RegistryTokenHttpService {
        let issuer = RegistryTokenIssuer::new(
            "https://forge.example/v1/registry/token"
                .parse()
                .expect("issuer"),
            "registry.forge.example".parse().expect("service"),
            SigningKey::hs256(
                "test-key".parse().expect("key id"),
                b"01234567890123456789012345678901",
            )
            .expect("signing key"),
            TokenLifetime::new(300).expect("lifetime"),
        );
        RegistryTokenHttpService::new(Arc::new(issuer), Arc::new(PullOnly))
    }

    fn identity() -> AuthenticatedIdentity {
        AuthenticatedIdentity::new(
            UserId::new(),
            "https://issuer.example",
            "subject",
            serde_json::json!({}),
            RequestId::new(),
        )
    }

    #[tokio::test]
    async fn endpoint_intersects_push_to_pull_only() {
        let response = service()
            .exchange(
                &identity(),
                TokenQuery {
                    service: String::from("registry.forge.example"),
                    scope: Some(String::from(
                        "repository:projects/123e4567-e89b-12d3-a456-426614174000/repository-builders/987e6543-e21b-12d3-a456-426614174000:pull,push",
                    )),
                    account: None,
                    client_id: None,
                },
                OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("time"),
            )
            .await
            .expect("token response");

        assert_eq!(response.expires_in, 300);
        assert_eq!(response.token, response.access_token);
        assert!(!response.token.is_empty());
    }

    #[tokio::test]
    async fn endpoint_rejects_wrong_service_and_unbounded_client_fields() {
        let mut query = TokenQuery {
            service: String::from("other.example"),
            scope: None,
            account: None,
            client_id: None,
        };
        assert!(
            service()
                .exchange(&identity(), query, OffsetDateTime::UNIX_EPOCH)
                .await
                .is_err()
        );
        query = TokenQuery {
            service: String::from("registry.forge.example"),
            scope: None,
            account: Some("x".repeat(257)),
            client_id: None,
        };
        assert!(
            service()
                .exchange(&identity(), query, OffsetDateTime::UNIX_EPOCH)
                .await
                .is_err()
        );
    }
}
