#![cfg(feature = "test-fixtures")]

//! Real daemon transport proof for the browser-session lifecycle.
//!
//! The sibling support fixture owns the disposable application configuration
//! and database/NATS cleanup boundary without changing the golden test or
//! introducing a second production fixture implementation.

#[path = "../support/browser_session_fixture.rs"]
mod app_fixture;

// The integration crate is test-only; keep its transport and database probes
// outside the production architecture surface while retaining the composition
// boundary for generated Connect types.
#[cfg(test)]
mod browser_session_transport {
    use super::app_fixture::{app_config, cleanup_nats, require_disposable_nats};
    use connectrpc::{
        Protocol,
        client::{CallOptions, ClientConfig, Http2Connection, SharedHttp2Connection},
        error::ErrorCode,
    };
    use hephaestus_app::{HephaestusApp, rpc::MediatorAuthenticator};
    use http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use identity_domain::{BrowserSessionSid, browser_session_sid_digest};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use rpc_proto::{
        connect::hephaestus::{
            identity::v1::IdentityServiceClient, organization::v1::OrganizationServiceClient,
        },
        messages::hephaestus::{
            common::v1::{OpaqueId, RequestContext},
            identity::v1::{CreateBrowserSessionRequest, RevokeBrowserSessionRequest},
            organization::v1::ListOrganizationsRequest,
        },
    };
    use serial_test::serial;
    use sqlx::{PgPool, postgres::PgPoolOptions};
    use std::time::{SystemTime, UNIX_EPOCH};
    use time::OffsetDateTime;
    use url::Url;
    use uuid::Uuid;

    const SECRET: &[u8] = b"service-log-retention-test-signing-secret";
    const ISSUER: &str = "https://browser-session-rpc.invalid";
    const AUDIENCE_CREATE: &str = "/hephaestus.identity.v1.IdentityService/CreateBrowserSession";
    const AUDIENCE_REVOKE: &str = "/hephaestus.identity.v1.IdentityService/RevokeBrowserSession";
    const AUDIENCE_ORGANIZATIONS: &str =
        "/hephaestus.organization.v1.OrganizationService/ListOrganizations";

    fn request_context(key: &str) -> RequestContext {
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

    fn now_seconds() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock is after Unix epoch")
            .as_secs()
            .try_into()
            .expect("test clock fits signed seconds")
    }

    fn bootstrap_token(issuer: &str, subject: &str) -> String {
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

    fn session_token(user_id: Uuid, sid: BrowserSessionSid, audience: &str) -> String {
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

    async fn seed_identity(pool: &PgPool, suffix: &str) -> (Uuid, Uuid, String) {
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

    async fn clients(
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

    fn authorization(token: &str) -> CallOptions {
        CallOptions::default().with_header("authorization", format!("Bearer {token}"))
    }

    fn require_disposable_postgres(database_url: &str) {
        let parsed = Url::parse(database_url).expect("parse PostgreSQL test URL");
        assert_eq!(parsed.scheme(), "postgres");
        assert!(
            matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1")),
            "browser-session test refuses a non-loopback PostgreSQL URL"
        );
    }

    async fn count_event(pool: &PgPool, event_id: Uuid) -> i64 {
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

    async fn identity_census(pool: &PgPool, user_id: Uuid) -> (i64, i64, i64) {
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

    fn assert_signed_session_token(token: &str, audience: &str) {
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[serial]
    async fn production_browser_session_create_protect_revoke_and_replay() {
        if std::env::var("REAL_APP_BROWSER_SESSION_RPC").as_deref() != Ok("1") {
            eprintln!("SKIPPED browser-session RPC lifecycle: set REAL_APP_BROWSER_SESSION_RPC=1");
            return;
        }
        let parent_url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL")
            .expect("HEPHAESTUS_POSTGRES_TEST_URL is required");
        let nats_url = std::env::var("HEPHAESTUS_NATS_TEST_URL")
            .expect("HEPHAESTUS_NATS_TEST_URL is required");
        require_disposable_postgres(&parent_url);
        require_disposable_nats(&nats_url);
        let database_url = parent_url;
        let seed_pool = PgPoolOptions::new()
            .max_connections(3)
            .connect(&database_url)
            .await
            .expect("connect browser-session seed pool");
        sqlx::migrate!("../../migrations")
            .run(&seed_pool)
            .await
            .expect("apply browser-session migrations");
        let (user_id, organization_id, issuer) = seed_identity(&seed_pool, "owner").await;
        let (other_user_id, _other_organization_id, other_issuer) =
            seed_identity(&seed_pool, "other").await;
        let subject = String::from("subject-owner");
        let other_subject = String::from("subject-other");
        let sid = BrowserSessionSid::new();
        let other_sid = BrowserSessionSid::new();
        let root = tempfile::tempdir().expect("browser-session application root");
        let app = HephaestusApp::build(app_config(
            database_url.clone(),
            nats_url.clone(),
            root.path(),
        ))
        .await
        .expect("build production browser-session application")
        .start()
        .await
        .expect("start production browser-session application");
        let (identity, organizations) = clients(app.http_addr()).await;

        let bootstrap = bootstrap_token(&issuer, &subject);
        let wrong_subject = identity
            .create_browser_session_with_options(
                CreateBrowserSessionRequest {
                    context: request_context("wrong-subject").into(),
                    issuer: issuer.clone(),
                    subject: String::from("wrong-subject"),
                    sid: sid
                        .to_protocol_string()
                        .parse::<Uuid>()
                        .expect("sid UUID")
                        .into_bytes()
                        .to_vec(),
                    ..Default::default()
                },
                authorization(&bootstrap),
            )
            .await
            .expect_err("bootstrap subject mismatch must be rejected");
        assert_eq!(wrong_subject.code, ErrorCode::Unauthenticated);

        let malformed_sid = identity
            .create_browser_session_with_options(
                CreateBrowserSessionRequest {
                    context: request_context("malformed-sid").into(),
                    issuer: issuer.clone(),
                    subject: subject.clone(),
                    sid: vec![1, 2, 3],
                    ..Default::default()
                },
                authorization(&bootstrap),
            )
            .await
            .expect_err("malformed SID must be rejected");
        assert_eq!(malformed_sid.code, ErrorCode::InvalidArgument);

        let create_context = request_context("create-owner");
        let created = identity
            .create_browser_session_with_options(
                CreateBrowserSessionRequest {
                    context: create_context.clone().into(),
                    issuer: issuer.clone(),
                    subject: subject.clone(),
                    sid: sid
                        .to_protocol_string()
                        .parse::<Uuid>()
                        .expect("sid UUID")
                        .into_bytes()
                        .to_vec(),
                    ..Default::default()
                },
                authorization(&bootstrap),
            )
            .await
            .expect("create browser session")
            .into_owned();
        assert_eq!(
            created.user_id.as_option().map(|value| value.value.clone()),
            Some(user_id.to_string())
        );
        let created_session_id = created
            .session_id
            .as_option()
            .expect("created session ID")
            .value
            .parse::<Uuid>()
            .expect("created session ID UUID");
        assert_ne!(created_session_id, Uuid::nil());
        assert!(created.expires_at.as_option().is_some());
        let creation_receipt = created.receipt.as_option().expect("creation receipt");
        let creation_event_id = creation_receipt
            .event_id
            .as_option()
            .expect("creation receipt event ID")
            .value
            .parse::<Uuid>()
            .expect("creation event ID UUID");
        assert_eq!(count_event(&seed_pool, creation_event_id).await, 1);
        assert!(!format!("{created:?}").contains(&sid.to_protocol_string()));
        let created_expiry = created.expires_at.as_option().expect("creation expiry");
        let now = OffsetDateTime::now_utc().unix_timestamp();
        assert!(created_expiry.seconds > now + 11 * 60 * 60);
        assert!(created_expiry.seconds <= now + 13 * 60 * 60);
        let stored_expiry: OffsetDateTime =
            sqlx::query_scalar("SELECT expires_at FROM human_browser_sessions WHERE id = $1")
                .bind(created_session_id)
                .fetch_one(&seed_pool)
                .await
                .expect("read created session expiry");
        assert_eq!(created_expiry.seconds, stored_expiry.unix_timestamp());
        let create_request_id = create_context
            .request_id
            .as_option()
            .expect("creation request ID")
            .value
            .parse::<Uuid>()
            .expect("creation request ID UUID");
        let creation_actor_request: (Uuid, Uuid) =
            sqlx::query_as("SELECT actor_id, request_id FROM application_events WHERE id = $1")
                .bind(creation_event_id)
                .fetch_one(&seed_pool)
                .await
                .expect("read creation event actor and request");
        assert_eq!(creation_actor_request, (user_id, create_request_id));

        let normal_token = session_token(user_id, sid, AUDIENCE_ORGANIZATIONS);
        assert_signed_session_token(&normal_token, AUDIENCE_ORGANIZATIONS);
        let organizations_before = organizations
            .list_organizations_with_options(
                ListOrganizationsRequest::default(),
                authorization(&normal_token),
            )
            .await
            .expect("active browser session may call ordinary protected RPC")
            .into_owned();
        assert!(
            organizations_before
                .organizations
                .iter()
                .any(|organization| organization
                    .id
                    .as_option()
                    .is_some_and(|id| id.value == organization_id.to_string()))
        );

        let other_created = identity
            .create_browser_session_with_options(
                CreateBrowserSessionRequest {
                    context: request_context("create-other").into(),
                    issuer: other_issuer,
                    subject: other_subject,
                    sid: other_sid
                        .to_protocol_string()
                        .parse::<Uuid>()
                        .expect("other SID UUID")
                        .into_bytes()
                        .to_vec(),
                    ..Default::default()
                },
                authorization(&bootstrap_token(
                    &format!("{ISSUER}/other"),
                    "subject-other",
                )),
            )
            .await
            .expect("create second browser session")
            .into_owned();
        assert_eq!(
            other_created
                .user_id
                .as_option()
                .map(|value| value.value.clone()),
            Some(other_user_id.to_string())
        );

        let before_revoke = identity_census(&seed_pool, user_id).await;
        let revoke_context = request_context("revoke-owner");
        let revoke_request_id = revoke_context
            .request_id
            .as_option()
            .expect("revocation request ID")
            .value
            .parse::<Uuid>()
            .expect("revocation request ID UUID");
        let revoke = identity
            .revoke_browser_session_with_options(
                RevokeBrowserSessionRequest {
                    context: revoke_context.clone().into(),
                    ..Default::default()
                },
                authorization(&session_token(user_id, sid, AUDIENCE_REVOKE)),
            )
            .await
            .expect("revoke browser session")
            .into_owned();
        let revoke_receipt = revoke.receipt.as_option().expect("revoke receipt");
        let revoke_event_id = revoke_receipt
            .event_id
            .as_option()
            .expect("revoke receipt event ID")
            .value
            .parse::<Uuid>()
            .expect("revoke event ID UUID");
        assert_eq!(count_event(&seed_pool, revoke_event_id).await, 1);
        assert_eq!(
            identity_census(&seed_pool, user_id).await,
            (
                before_revoke.0 + 1,
                before_revoke.1 + 1,
                before_revoke.2 + 1
            )
        );
        let revoke_actor_request: (Uuid, Uuid) =
            sqlx::query_as("SELECT actor_id, request_id FROM application_events WHERE id = $1")
                .bind(revoke_event_id)
                .fetch_one(&seed_pool)
                .await
                .expect("read revocation event actor and request");
        assert_eq!(revoke_actor_request, (user_id, revoke_request_id));

        let replay = identity
            .revoke_browser_session_with_options(
                RevokeBrowserSessionRequest {
                    context: revoke_context.into(),
                    ..Default::default()
                },
                authorization(&session_token(user_id, sid, AUDIENCE_REVOKE)),
            )
            .await
            .expect("inactive-session revoke replay remains successful")
            .into_owned();
        assert_eq!(replay.receipt, revoke.receipt);
        assert_eq!(count_event(&seed_pool, revoke_event_id).await, 1);
        assert_eq!(
            identity_census(&seed_pool, user_id).await,
            (
                before_revoke.0 + 1,
                before_revoke.1 + 1,
                before_revoke.2 + 1,
            )
        );

        let denied = organizations
            .list_organizations_with_options(
                ListOrganizationsRequest::default(),
                authorization(&normal_token),
            )
            .await
            .expect_err("revoked browser session must fail ordinary RPC auth");
        assert_eq!(denied.code, ErrorCode::Unauthenticated);
        assert_signed_session_token(&normal_token, AUDIENCE_ORGANIZATIONS);

        let other_token = session_token(other_user_id, other_sid, AUDIENCE_ORGANIZATIONS);
        let _ = organizations
            .list_organizations_with_options(
                ListOrganizationsRequest::default(),
                authorization(&other_token),
            )
            .await
            .expect("revoking one user's session must not affect another user")
            .into_owned();
        let other_revoked: Option<OffsetDateTime> = sqlx::query_scalar(
            "SELECT revoked_at FROM human_browser_sessions
                 WHERE user_id = $1 AND sid_digest = $2",
        )
        .bind(other_user_id)
        .bind(browser_session_sid_digest(other_sid).as_bytes().to_vec())
        .fetch_one(&seed_pool)
        .await
        .expect("inspect other user's session");
        assert!(other_revoked.is_none());

        app.shutdown()
            .await
            .expect("shutdown browser-session application");
        seed_pool.close().await;
        cleanup_nats(&nats_url).await;
    }
}
