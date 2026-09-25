#![cfg(feature = "test-fixtures")]

//! Production Connect coverage for the released UI installation boundary.

#[path = "../support/browser_session_fixture.rs"]
mod app_fixture;

#[cfg(test)]
#[path = "ui_installation_rpc/scenario_handoff.rs"]
mod ui_installation_scenario_handoff;
#[cfg(test)]
#[path = "ui_installation_rpc/scenario_install.rs"]
mod ui_installation_scenario_install;
#[cfg(test)]
#[path = "ui_installation_rpc/scenario_lifecycle.rs"]
mod ui_installation_scenario_lifecycle;
#[cfg(test)]
#[path = "ui_installation_rpc/scenario_listing.rs"]
mod ui_installation_scenario_listing;
#[cfg(test)]
#[path = "ui_installation_rpc/scenario_state.rs"]
mod ui_installation_scenario_state;
#[cfg(test)]
#[path = "ui_installation_rpc/seed.rs"]
mod ui_installation_seed;
#[cfg(test)]
#[path = "ui_installation_rpc/seed_base.rs"]
mod ui_installation_seed_base;
#[cfg(test)]
#[path = "ui_installation_rpc/seed_releases.rs"]
mod ui_installation_seed_releases;
#[cfg(test)]
#[path = "ui_installation_rpc/seed_sessions.rs"]
mod ui_installation_seed_sessions;

#[cfg(test)]
mod ui_installation_transport {
    use super::app_fixture::{app_config, cleanup_nats, require_disposable_nats};
    use super::ui_installation_seed::seed_fixture;
    use connectrpc::{
        Protocol,
        client::{CallOptions, ClientConfig, Http2Connection, SharedHttp2Connection},
    };
    use hephaestus_app::HephaestusApp;
    use identity_domain::BrowserSessionSid;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use rpc_proto::{
        connect::hephaestus::release::v1::ReleaseServiceClient,
        messages::hephaestus::{
            common::v1::{MutationReceipt, OpaqueId, RequestContext},
            release::v1::{UiInstallationTarget, ui_installation_target},
        },
    };
    use serde_json::json;
    use serial_test::serial;
    use sqlx::{PgPool, postgres::PgPoolOptions};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;
    use tracing_subscriber::{EnvFilter, fmt};
    use uuid::Uuid;

    const SECRET: &[u8] = b"service-log-retention-test-signing-secret";
    pub(crate) const UI_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/InstallUi";
    pub(crate) const ACTIVATE_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/ActivateUi";
    pub(crate) const ROLLBACK_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/RollbackUi";
    pub(crate) const DISABLE_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/DisableUi";
    pub(crate) const REMOVE_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/RemoveUi";
    pub(crate) const LIST_AUDIENCE: &str =
        "/hephaestus.release.v1.ReleaseService/ListUiInstallations";
    pub(crate) const HANDOFF_AUDIENCE: &str =
        "/hephaestus.release.v1.ReleaseService/CreateUiBrowserHandoff";
    pub(crate) const HANDOFF_SENTINEL_TEXT: &str = "handoff-secret-sentinel-9e25143x";
    pub(crate) const HANDOFF_SENTINEL: &[u8] = HANDOFF_SENTINEL_TEXT.as_bytes();

    pub(crate) type InvalidSecretAudit = (
        Uuid,
        String,
        String,
        String,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
    );

    pub(crate) fn opaque(id: Uuid) -> OpaqueId {
        OpaqueId {
            value: id.to_string(),
            ..Default::default()
        }
    }

    pub(crate) fn request_context(key: &str) -> RequestContext {
        RequestContext {
            request_id: opaque(Uuid::new_v4()).into(),
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
            .expect("test clock fits i64")
    }

    pub(crate) fn session_token(user_id: Uuid, sid: BrowserSessionSid, audience: &str) -> String {
        session_token_for_lifetime(user_id, sid, audience, 25)
    }

    pub(crate) fn session_token_for_lifetime(
        user_id: Uuid,
        sid: BrowserSessionSid,
        audience: &str,
        lifetime_seconds: i64,
    ) -> String {
        let now = now_seconds();
        encode(
            &Header::new(Algorithm::HS256),
            &json!({
                "iss": "hephaestus-web-mediator",
                "sub": user_id.to_string(),
                "aud": audience,
                "iat": now,
                "nbf": now,
                "exp": now + lifetime_seconds,
                "jti": Uuid::new_v4().to_string(),
                "sid": sid.to_protocol_string(),
            }),
            &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(SECRET)),
        )
        .expect("sign browser session assertion")
    }

    pub(crate) fn authorization(token: &str) -> CallOptions {
        CallOptions::default().with_header("authorization", format!("Bearer {token}"))
    }

    pub(crate) fn project_target(project_id: Uuid) -> UiInstallationTarget {
        UiInstallationTarget {
            target: Some(ui_installation_target::Target::ProjectId(Box::new(opaque(
                project_id,
            )))),
            ..Default::default()
        }
    }

    pub(crate) fn organization_target() -> UiInstallationTarget {
        UiInstallationTarget {
            target: Some(ui_installation_target::Target::Global(Box::default())),
            ..Default::default()
        }
    }

    pub(crate) fn repository_target(repository_id: Uuid) -> UiInstallationTarget {
        UiInstallationTarget {
            target: Some(ui_installation_target::Target::RepositoryId(Box::new(
                opaque(repository_id),
            ))),
            ..Default::default()
        }
    }

    async fn release_client(
        address: std::net::SocketAddr,
    ) -> ReleaseServiceClient<SharedHttp2Connection> {
        let uri: axum::http::Uri = format!("http://{address}").parse().expect("UI RPC URI");
        let connection = Http2Connection::connect_plaintext(uri.clone())
            .await
            .expect("UI RPC connection")
            .shared(4);
        ReleaseServiceClient::new(
            connection,
            ClientConfig::new(uri).with_protocol(Protocol::Connect),
        )
    }

    pub(crate) async fn assert_receipt_scope(
        pool: &PgPool,
        receipt: &MutationReceipt,
        expected_aggregate: &str,
        expected_scope: &str,
    ) {
        let event_id = receipt
            .event_id
            .as_option()
            .expect("mutation receipt event id")
            .value
            .parse::<Uuid>()
            .expect("mutation receipt event UUID");
        let scope: (String, String) = sqlx::query_as(
            "SELECT aggregate_type, scope_kind FROM application_events WHERE id = $1",
        )
        .bind(event_id)
        .fetch_one(pool)
        .await
        .expect("load mutation receipt scope");
        assert_eq!(
            scope,
            (expected_aggregate.to_owned(), expected_scope.to_owned())
        );
    }

    #[allow(clippy::large_stack_frames)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[serial]
    #[ignore = "requires disposable PostgreSQL/NATS; use scripts/test-ui-installation-rpc.sh"]
    // Keep the authenticated mutation sequence together so replay, CAS, and
    // no-row assertions share one production installation and generation chain.
    #[allow(clippy::too_many_lines)]
    async fn production_ui_installation_rpc_matrix() {
        // Keep bounded transport-audit failures visible in the CI test log.
        // `try_init` preserves compatibility with a harness that already owns
        // the process-wide subscriber.
        let _ = fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new("hephaestus_app=warn")),
            )
            .with_test_writer()
            .try_init();
        assert_eq!(
            std::env::var("REAL_UI_INSTALLATION_RPC"),
            Ok(String::from("1")),
            "set REAL_UI_INSTALLATION_RPC=1 for this real production-router proof"
        );
        let database_url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL")
            .expect("HEPHAESTUS_POSTGRES_TEST_URL is required");
        let nats_url = std::env::var("HEPHAESTUS_NATS_TEST_URL")
            .expect("HEPHAESTUS_NATS_TEST_URL is required");
        require_disposable_nats(&nats_url);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&database_url)
            .await
            .expect("connect UI RPC seed pool");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("apply UI RPC migrations");
        let fixture = seed_fixture(&pool).await;
        let root = tempdir().expect("UI RPC application root");
        let app = HephaestusApp::build(app_config(
            database_url.clone(),
            nats_url.clone(),
            root.path(),
        ))
        .await
        .expect("build production UI RPC application")
        .start()
        .await
        .expect("start production UI RPC application");
        let release = release_client(app.http_addr()).await;
        let state = super::ui_installation_scenario_install::run(&pool, &release, &fixture).await;
        super::ui_installation_scenario_listing::run(&release, &fixture, &state).await;
        super::ui_installation_scenario_handoff::run(&pool, &release, &fixture, &app, &state).await;
        super::ui_installation_scenario_lifecycle::run(&pool, &release, &fixture, &state).await;
        println!(
            "REAL_UI_INSTALLATION_RPC=1 install_replay=1 organization=1 repository=1 receipts=1 list=1 cursor_actor=1 cursor_org=1 cursor_target=1 cursor_tamper=1 handoff_secret=32 handoff_parent_redacted=1 lifecycle_all5=1 stale_cas=1 wrong_tenant=1 wrong_audience=1 expired=1 revoked=1"
        );
        app.shutdown().await.expect("shutdown UI RPC application");
        pool.close().await;
        cleanup_nats(&nats_url).await;
    }
}
