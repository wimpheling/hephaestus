use axum::{body::Body, extract::State, http::Request, middleware::Next, response::Response};
use http::{HeaderMap, StatusCode, header::AUTHORIZATION};
use identity_domain::UserId;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use std::{collections::HashSet, str::FromStr};
use time::OffsetDateTime;
use uuid::Uuid;

const ISSUER: &str = "hephaestus-web-mediator";
const BOOTSTRAP_SUBJECT: &str = "hephaestus-web-mediator";
const BOOTSTRAP_ACTOR_KIND: &str = "verified_oidc_bootstrap";
const MAX_LIFETIME_SECONDS: i64 = 30;
const CLOCK_SKEW_SECONDS: i64 = 5;

/// Authenticated mediator subject safe to convert into application identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediatorPrincipal {
    /// Internal user selected by the trusted mediator assertion.
    pub user_id: UserId,
    /// Unique assertion identifier retained only for audit correlation.
    pub assertion_id: Uuid,
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
        })
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
pub async fn mediator_identity_middleware(
    State(authenticator): State<MediatorAuthenticator>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if !requires_mediator_auth(request.uri().path()) {
        return next.run(request).await;
    }
    let audience = request.uri().path().to_owned();
    let Ok(principal) = authenticator.authenticate(request.headers(), &audience) else {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::UNAUTHORIZED;
        return response;
    };
    request
        .extensions_mut()
        .insert(identity_domain::AuthenticatedIdentity::new(
            principal.user_id,
            ISSUER,
            principal.user_id.to_string(),
            serde_json::json!({"mediator": "phoenix", "assertion_id": principal.assertion_id}),
            identity_domain::RequestId::from_uuid(principal.assertion_id),
        ));
    next.run(request).await
}

fn requires_mediator_auth(path: &str) -> bool {
    path.starts_with("/hephaestus.")
}

#[derive(Deserialize)]
struct MediatorClaims {
    sub: String,
    jti: String,
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
        BOOTSTRAP_ACTOR_KIND, BOOTSTRAP_SUBJECT, BootstrapIdentity, CLOCK_SKEW_SECONDS, ISSUER,
        MediatorAuthenticator, requires_mediator_auth,
    };
    use crate::rpc::mediator_signing_key;
    use http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde::Serialize;
    use time::OffsetDateTime;
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
    fn middleware_authenticates_only_connect_paths() {
        assert!(requires_mediator_auth(
            "/hephaestus.agent.v1.AgentService/ImportAgent"
        ));
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

    fn assertion(
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
            },
            &EncodingKey::from_secret(key),
        )
        .expect("encode assertion")
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
