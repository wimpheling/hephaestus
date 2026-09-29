use connectrpc::{
    Protocol,
    client::{CallOptions, ClientConfig, Http2Connection, SharedHttp2Connection},
};
use hephaestus_app::rpc::MediatorAuthenticator;
use http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
use identity_domain::BrowserSessionSid;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rpc_proto::{
    connect::hephaestus::{
        identity::v1::IdentityServiceClient, organization::v1::OrganizationServiceClient,
    },
    messages::hephaestus::common::v1::{OpaqueId, RequestContext},
};
use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;
use uuid::Uuid;

pub const SECRET: &[u8] = b"service-log-retention-test-signing-secret";
pub const ISSUER: &str = "https://browser-session-rpc.invalid";
pub const AUDIENCE_CREATE: &str = "/hephaestus.identity.v1.IdentityService/CreateBrowserSession";
pub const AUDIENCE_REVOKE: &str = "/hephaestus.identity.v1.IdentityService/RevokeBrowserSession";
pub const AUDIENCE_ORGANIZATIONS: &str =
    "/hephaestus.organization.v1.OrganizationService/ListOrganizations";

pub fn request_context(key: &str) -> RequestContext {
    RequestContext {
        request_id: OpaqueId {
            value: Uuid::new_v4().to_string(),
            ..Default::default()
        }
        .into(),
        idempotency_key: key.to_owned(),
        ..Default::default()
    }
}

pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after Unix epoch")
        .as_secs()
        .try_into()
        .expect("test clock fits signed seconds")
}

pub fn bootstrap_token(issuer: &str, subject: &str) -> String {
    let now = now_seconds();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": "hephaestus-web-mediator",
            "sub": "hephaestus-web-mediator",
            "aud": AUDIENCE_CREATE,
            "iat": now,
            "nbf": now,
            "exp": now + 25,
            "jti": Uuid::new_v4().to_string(),
            "actor_kind": "verified_oidc_bootstrap",
            "oidc_iss": issuer,
            "oidc_sub": subject,
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(SECRET)),
    )
    .expect("sign browser-session bootstrap assertion")
}

pub fn session_token(user_id: Uuid, sid: BrowserSessionSid, audience: &str) -> String {
    let now = now_seconds();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": "hephaestus-web-mediator",
            "sub": user_id.to_string(),
            "aud": audience,
            "iat": now,
            "nbf": now,
            "exp": now + 25,
            "jti": Uuid::new_v4().to_string(),
            "sid": sid.to_protocol_string(),
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(SECRET)),
    )
    .expect("sign browser-session assertion")
}

pub async fn seed_identity(pool: &PgPool, suffix: &str) -> (Uuid, Uuid, String) {
    let user_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let issuer = format!("{ISSUER}/{suffix}");
    let subject = format!("subject-{suffix}");
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user_id)
        .bind(format!("Browser session {suffix}"))
        .execute(pool)
        .await
        .expect("seed browser-session user");
    sqlx::query(
        "INSERT INTO external_identities (user_id, issuer, subject, provider_metadata)
                 VALUES ($1, $2, $3, '{}'::jsonb)",
    )
    .bind(user_id)
    .bind(&issuer)
    .bind(&subject)
    .execute(pool)
    .await
    .expect("seed browser-session external identity");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("Browser session org {suffix}"))
        .execute(pool)
        .await
        .expect("seed browser-session organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
                 VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed browser-session organization owner");
    (user_id, organization_id, issuer)
}

pub async fn clients(
    address: std::net::SocketAddr,
) -> (
    IdentityServiceClient<SharedHttp2Connection>,
    OrganizationServiceClient<SharedHttp2Connection>,
) {
    let uri: axum::http::Uri = format!("http://{address}")
        .parse()
        .expect("browser-session RPC URI");
    let connection = Http2Connection::connect_plaintext(uri.clone())
        .await
        .expect("browser-session RPC connection")
        .shared(4);
    let config = ClientConfig::new(uri).with_protocol(Protocol::Connect);
    (
        IdentityServiceClient::new(connection.clone(), config.clone()),
        OrganizationServiceClient::new(connection, config),
    )
}

pub fn authorization(token: &str) -> CallOptions {
    CallOptions::default().with_header("authorization", format!("Bearer {token}"))
}

pub fn require_disposable_postgres(database_url: &str) {
    let parsed = Url::parse(database_url).expect("parse PostgreSQL test URL");
    assert_eq!(parsed.scheme(), "postgres");
    assert!(
        matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1")),
        "browser-session test refuses a non-loopback PostgreSQL URL"
    );
}

pub async fn count_event(pool: &PgPool, event_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM application_events
                 WHERE id = $1 AND aggregate_type = 'identity_profile'
                   AND event_type = 'identity.profile_changed'",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("count identity mutation event")
}

pub async fn identity_census(pool: &PgPool, user_id: Uuid) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT
                    (SELECT count(*) FROM application_events
                     WHERE scope_kind = 'identity' AND scope_id = $1
                       AND event_type = 'identity.profile_changed'),
                    (SELECT count(*)
                     FROM product_event_outbox AS outbox
                     JOIN application_events AS event ON event.id = outbox.event_id
                     WHERE event.scope_kind = 'identity' AND event.scope_id = $1
                       AND event.event_type = 'identity.profile_changed'),
                    (SELECT count(*) FROM human_browser_session_revocations
                     WHERE user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .expect("read identity event and revocation census")
}

pub fn assert_signed_session_token(token: &str, audience: &str) {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}"))
            .expect("session token authorization header"),
    );
    MediatorAuthenticator::new(&hephaestus_app::rpc::mediator_signing_key(SECRET))
        .authenticate(&headers, audience)
        .expect("session JWT remains cryptographically valid before active lookup");
}
