#![cfg(feature = "test-fixtures")]

//! Production Connect coverage for the released UI installation boundary.

#[path = "../support/browser_session_fixture.rs"]
mod app_fixture;

#[cfg(test)]
mod ui_installation_transport {
    use super::app_fixture::{app_config, cleanup_nats, require_disposable_nats};
    use connectrpc::{
        Protocol,
        client::{CallOptions, ClientConfig, Http2Connection, SharedHttp2Connection},
        error::ErrorCode,
    };
    use hephaestus_app::HephaestusApp;
    use identity_domain::{
        AuthenticatedIdentity, BrowserSessionSid, RequestId, UserId,
        browser_session_identity_binding_digest, browser_session_sid_digest,
    };
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use rpc_proto::{
        connect::hephaestus::release::v1::ReleaseServiceClient,
        messages::hephaestus::{
            common::v1::{MutationReceipt, OpaqueId, PageRequest, RequestContext},
            release::v1::{
                ActivateUiRequest, CreateUiBrowserHandoffRequest, DisableUiRequest,
                InstallUiRequest, ListUiInstallationsRequest, RemoveUiRequest, RollbackUiRequest,
                UiInstallationTarget, ui_installation_target,
            },
        },
    };
    use serde_json::json;
    use serial_test::serial;
    use sqlx::{PgPool, postgres::PgPoolOptions};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;
    use time::OffsetDateTime;
    use uuid::Uuid;

    const SECRET: &[u8] = b"service-log-retention-test-signing-secret";
    const ISSUER: &str = "https://ui-rpc-test.invalid";
    const UI_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/InstallUi";
    const ACTIVATE_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/ActivateUi";
    const ROLLBACK_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/RollbackUi";
    const DISABLE_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/DisableUi";
    const REMOVE_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/RemoveUi";
    const LIST_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/ListUiInstallations";
    const HANDOFF_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/CreateUiBrowserHandoff";

    type InvalidSecretAudit = (
        Uuid,
        String,
        String,
        String,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
    );

    #[derive(Clone, Copy)]
    struct Fixture {
        user_id: Uuid,
        organization_id: Uuid,
        foreign_organization_id: Uuid,
        project_id: Uuid,
        repository_id: Uuid,
        release_id: Uuid,
        global_release_id: Uuid,
        repository_release_id: Uuid,
        parent_session_id: Uuid,
        sid: BrowserSessionSid,
        second_user_id: Uuid,
        second_sid: BrowserSessionSid,
    }

    fn opaque(id: Uuid) -> OpaqueId {
        OpaqueId {
            value: id.to_string(),
            ..Default::default()
        }
    }

    fn request_context(key: &str) -> RequestContext {
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

    fn session_token(user_id: Uuid, sid: BrowserSessionSid, audience: &str) -> String {
        session_token_for_lifetime(user_id, sid, audience, 25)
    }

    fn session_token_for_lifetime(
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

    fn authorization(token: &str) -> CallOptions {
        CallOptions::default().with_header("authorization", format!("Bearer {token}"))
    }

    fn project_target(project_id: Uuid) -> UiInstallationTarget {
        UiInstallationTarget {
            target: Some(ui_installation_target::Target::ProjectId(Box::new(opaque(
                project_id,
            )))),
            ..Default::default()
        }
    }

    fn organization_target() -> UiInstallationTarget {
        UiInstallationTarget {
            target: Some(ui_installation_target::Target::Global(Box::default())),
            ..Default::default()
        }
    }

    fn repository_target(repository_id: Uuid) -> UiInstallationTarget {
        UiInstallationTarget {
            target: Some(ui_installation_target::Target::RepositoryId(Box::new(
                opaque(repository_id),
            ))),
            ..Default::default()
        }
    }

    // Keep the complete SQL fixture in one transaction-independent setup so the
    // production router proof exercises the same published rows as the adapter.
    // The fixture intentionally keeps all cross-scope rows adjacent for auditability.
    #[allow(clippy::cognitive_complexity)]
    #[allow(clippy::too_many_lines)]
    async fn seed_fixture(pool: &PgPool) -> Fixture {
        let user_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let foreign_organization_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let repository_id = Uuid::new_v4();
        let second_user_id = Uuid::new_v4();
        let receive_id = Uuid::new_v4();
        let build_id = Uuid::new_v4();
        let source_revision_id = Uuid::new_v4();
        let release_id = Uuid::new_v4();
        let global_release_id = Uuid::new_v4();
        let repository_release_id = Uuid::new_v4();
        let artifact_id = Uuid::new_v4();
        let global_artifact_id = Uuid::new_v4();
        let repository_artifact_id = Uuid::new_v4();
        let parent_session_id = Uuid::new_v4();
        let second_parent_session_id = Uuid::new_v4();
        let sid = BrowserSessionSid::new();
        let second_sid = BrowserSessionSid::new();
        let issuer = String::from(ISSUER);
        let subject = String::from("ui-rpc-owner");
        let identity = AuthenticatedIdentity::new(
            UserId::from_uuid(user_id),
            issuer.clone(),
            subject.clone(),
            json!({"email_verified": true}),
            RequestId::new(),
        );
        let issued_at = OffsetDateTime::now_utc();

        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(user_id)
            .bind("UI RPC owner")
            .execute(pool)
            .await
            .expect("seed UI RPC user");
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(second_user_id)
            .bind("UI RPC second admin")
            .execute(pool)
            .await
            .expect("seed second UI RPC user");
        sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
            .bind(organization_id)
            .bind("UI RPC organization")
            .execute(pool)
            .await
            .expect("seed UI RPC organization");
        sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
            .bind(foreign_organization_id)
            .bind("UI RPC foreign organization")
            .execute(pool)
            .await
            .expect("seed foreign UI RPC organization");
        sqlx::query(
            "INSERT INTO organization_members (organization_id, user_id, role)
             VALUES ($1, $2, 'owner')",
        )
        .bind(organization_id)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("seed UI RPC organization owner");
        sqlx::query(
            "INSERT INTO organization_members (organization_id, user_id, role)
             VALUES ($1, $2, 'admin')",
        )
        .bind(organization_id)
        .bind(second_user_id)
        .execute(pool)
        .await
        .expect("seed second UI RPC organization admin");
        sqlx::query(
            "INSERT INTO organization_members (organization_id, user_id, role)
             VALUES ($1, $2, 'admin')",
        )
        .bind(foreign_organization_id)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("seed cross-tenant UI RPC admin");
        sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
            .bind(project_id)
            .bind(organization_id)
            .bind("ui-rpc-project")
            .execute(pool)
            .await
            .expect("seed UI RPC project");
        sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
            .bind(project_id)
            .bind(user_id)
            .execute(pool)
            .await
            .expect("seed UI RPC project maintainer");
        sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
            .bind(project_id)
            .bind(second_user_id)
            .execute(pool)
            .await
            .expect("seed second UI RPC project maintainer");
        sqlx::query(
            "INSERT INTO repositories (id, project_id, name, default_branch, is_public)
             VALUES ($1, $2, $3, 'refs/heads/main', false)",
        )
        .bind(repository_id)
        .bind(project_id)
        .bind("ui-rpc-repository")
        .execute(pool)
        .await
        .expect("seed UI RPC repository");
        sqlx::query(
            "INSERT INTO git_receives (id, repository_id, actor_id, principal, status, accepted_at)
             VALUES ($1, $2, $3, 'ui-rpc-test', 'accepted', now())",
        )
        .bind(receive_id)
        .bind(repository_id)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("seed UI RPC receive");
        let commit = "b".repeat(40);
        sqlx::query(
            "INSERT INTO build_requests
             (id, repository_id, source_commit, source_ref, origin_receive_id,
              build_definition_hash, state, created_by)
             VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'succeeded', $6)",
        )
        .bind(build_id)
        .bind(repository_id)
        .bind(&commit)
        .bind(receive_id)
        .bind([1_u8; 32].as_slice())
        .bind(user_id)
        .execute(pool)
        .await
        .expect("seed UI RPC build");
        sqlx::query(
            "INSERT INTO ui_source_manifest_revisions
             (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
              actual_size_bytes, source_sha256, status, requires_gateways,
              normalized_ui_config, normalized_ui_hash, diagnostics)
             VALUES ($1, $2, $3, $4, 'blob', $5, 2, $6, 'valid', false, $7, $8, '[]')",
        )
        .bind(source_revision_id)
        .bind(repository_id)
        .bind(receive_id)
        .bind(&commit)
        .bind(&commit)
        .bind([2_u8; 32].as_slice())
        .bind(json!({"uis": []}))
        .bind([3_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("seed UI source revision");
        sqlx::query(
            "INSERT INTO build_request_ui_source_manifests
             (build_request_id, repository_id, source_commit, source_manifest_revision_id)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(build_id)
        .bind(repository_id)
        .bind(&commit)
        .bind(source_revision_id)
        .execute(pool)
        .await
        .expect("seed UI source capture");
        sqlx::query(
            "INSERT INTO releases
             (id, repository_id, version, source_commit, source_ref, build_request_id,
              build_definition_hash, configuration, configuration_hash, manifest_hash,
              state, publication_actor_id)
             VALUES ($1, $2, 'ui-rpc-v1', $3, 'refs/heads/main', $4, $5, $6, $7, $8,
                     'draft', $9)",
        )
        .bind(release_id)
        .bind(repository_id)
        .bind(&commit)
        .bind(build_id)
        .bind([4_u8; 32].as_slice())
        .bind(json!({"ui": true}))
        .bind([5_u8; 32].as_slice())
        .bind([6_u8; 32].as_slice())
        .bind(user_id)
        .execute(pool)
        .await
        .expect("seed UI release");
        for (id, version) in [
            (global_release_id, "ui-rpc-global"),
            (repository_release_id, "ui-rpc-repository"),
        ] {
            sqlx::query(
                "INSERT INTO releases
                 (id, repository_id, version, source_commit, source_ref, build_request_id,
                  build_definition_hash, configuration, configuration_hash, manifest_hash,
                  state, publication_actor_id)
                 VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, $7, $8, $9,
                         'draft', $10)",
            )
            .bind(id)
            .bind(repository_id)
            .bind(version)
            .bind(&commit)
            .bind(build_id)
            .bind([4_u8; 32].as_slice())
            .bind(json!({"ui": true}))
            .bind([5_u8; 32].as_slice())
            .bind([6_u8; 32].as_slice())
            .bind(user_id)
            .execute(pool)
            .await
            .expect("seed scoped UI release");
        }
        sqlx::query(
            "INSERT INTO release_ui_source_snapshots
             (release_id, build_request_id, source_manifest_revision_id)
             VALUES ($1, $2, $3)",
        )
        .bind(release_id)
        .bind(build_id)
        .bind(source_revision_id)
        .execute(pool)
        .await
        .expect("seed UI release source snapshot");
        for release in [global_release_id, repository_release_id] {
            sqlx::query(
                "INSERT INTO release_ui_source_snapshots
                 (release_id, build_request_id, source_manifest_revision_id)
                 VALUES ($1, $2, $3)",
            )
            .bind(release)
            .bind(build_id)
            .bind(source_revision_id)
            .execute(pool)
            .await
            .expect("seed scoped UI release source snapshot");
        }
        sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, 'assistant', 'project', 'Assistant', 'app', 'full_page',
                     'assistant', 'index.html', 1, 'no_store', 'static')",
        )
        .bind(release_id)
        .execute(pool)
        .await
        .expect("seed UI descriptor");
        sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, 'assistant', 'global', 'Assistant', 'app', 'full_page',
                     'assistant', 'index.html', 1, 'no_store', 'static')",
        )
        .bind(global_release_id)
        .execute(pool)
        .await
        .expect("seed global UI descriptor");
        sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, 'assistant', 'repository', 'Assistant', 'app', 'full_page',
                     'assistant', 'index.html', 1, 'no_store', 'static')",
        )
        .bind(repository_release_id)
        .execute(pool)
        .await
        .expect("seed repository UI descriptor");
        sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, 'console', 'global', 'Console', 'app', 'full_page',
                     'console', 'index.html', 1, 'no_store', 'static')",
        )
        .bind(global_release_id)
        .execute(pool)
        .await
        .expect("seed second global UI descriptor");
        sqlx::query(
            "INSERT INTO release_artifacts
             (id, release_id, path, kind, mode, content_hash, size_bytes, media_type, storage_key)
             VALUES ($1, $2, 'index.html', 'file', 420, $3, 19, 'text/html', $4)",
        )
        .bind(artifact_id)
        .bind(release_id)
        .bind([7_u8; 32].as_slice())
        .bind(Uuid::new_v4())
        .execute(pool)
        .await
        .expect("seed UI artifact");
        for (artifact, release) in [
            (global_artifact_id, global_release_id),
            (repository_artifact_id, repository_release_id),
        ] {
            sqlx::query(
                "INSERT INTO release_artifacts
                 (id, release_id, path, kind, mode, content_hash, size_bytes,
                  media_type, storage_key)
                 VALUES ($1, $2, 'index.html', 'file', 420, $3, 19,
                         'text/html', $4)",
            )
            .bind(artifact)
            .bind(release)
            .bind([7_u8; 32].as_slice())
            .bind(Uuid::new_v4())
            .execute(pool)
            .await
            .expect("seed scoped UI artifact");
        }
        sqlx::query(
            "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'assistant', 'index.html', $2, 'file', 'text/html')",
        )
        .bind(release_id)
        .bind(artifact_id)
        .execute(pool)
        .await
        .expect("seed UI static file");
        sqlx::query(
            "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'assistant', 'index.html', $2, 'file', 'text/html')",
        )
        .bind(global_release_id)
        .bind(global_artifact_id)
        .execute(pool)
        .await
        .expect("seed global assistant UI static file");
        sqlx::query(
            "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'console', 'index.html', $2, 'file', 'text/html')",
        )
        .bind(global_release_id)
        .bind(global_artifact_id)
        .execute(pool)
        .await
        .expect("seed global UI static file");
        sqlx::query(
            "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'assistant', 'index.html', $2, 'file', 'text/html')",
        )
        .bind(repository_release_id)
        .bind(repository_artifact_id)
        .execute(pool)
        .await
        .expect("seed repository UI static file");
        sqlx::query(
            "UPDATE releases SET state = 'published', published_at = now()
             WHERE id = ANY($1)",
        )
        .bind(vec![release_id, global_release_id, repository_release_id])
        .execute(pool)
        .await
        .expect("publish UI releases");
        sqlx::query(
            "INSERT INTO human_browser_sessions
             (id, sid_digest, creation_idempotency_id, creation_request_id,
              identity_binding_digest, user_id, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(parent_session_id)
        .bind(browser_session_sid_digest(sid).as_bytes().to_vec())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(
            browser_session_identity_binding_digest(&identity)
                .as_bytes()
                .to_vec(),
        )
        .bind(user_id)
        .bind(issued_at)
        .bind(issued_at + time::Duration::hours(1))
        .execute(pool)
        .await
        .expect("seed active parent browser session");
        let second_identity = AuthenticatedIdentity::new(
            UserId::from_uuid(second_user_id),
            issuer,
            String::from("ui-rpc-second-admin"),
            json!({"email_verified": true}),
            RequestId::new(),
        );
        sqlx::query(
            "INSERT INTO human_browser_sessions
             (id, sid_digest, creation_idempotency_id, creation_request_id,
              identity_binding_digest, user_id, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(second_parent_session_id)
        .bind(browser_session_sid_digest(second_sid).as_bytes().to_vec())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(
            browser_session_identity_binding_digest(&second_identity)
                .as_bytes()
                .to_vec(),
        )
        .bind(second_user_id)
        .bind(issued_at)
        .bind(issued_at + time::Duration::hours(1))
        .execute(pool)
        .await
        .expect("seed second active parent browser session");
        Fixture {
            user_id,
            organization_id,
            foreign_organization_id,
            project_id,
            repository_id,
            release_id,
            global_release_id,
            repository_release_id,
            parent_session_id,
            sid,
            second_user_id,
            second_sid,
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

    async fn assert_receipt_scope(
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
        let target = project_target(fixture.project_id);
        let install_token = session_token(fixture.user_id, fixture.sid, UI_AUDIENCE);
        let install_request = InstallUiRequest {
            context: request_context("ui-install").into(),
            organization_id: opaque(fixture.organization_id).into(),
            target: Some(target.clone()).into(),
            release_id: opaque(fixture.release_id).into(),
            ui_key: String::from("assistant"),
            ..Default::default()
        };
        let wrong_audience = release
            .install_ui_with_options(
                install_request.clone(),
                authorization(&session_token(fixture.user_id, fixture.sid, LIST_AUDIENCE)),
            )
            .await
            .expect_err("wrong procedure audience must be rejected");
        assert_eq!(wrong_audience.code, ErrorCode::Unauthenticated);
        let expired = release
            .install_ui_with_options(
                install_request.clone(),
                authorization(&session_token_for_lifetime(
                    fixture.user_id,
                    fixture.sid,
                    UI_AUDIENCE,
                    -1,
                )),
            )
            .await
            .expect_err("expired parent assertion must be rejected");
        assert_eq!(expired.code, ErrorCode::Unauthenticated);
        let inactive_parent = release
            .install_ui_with_options(
                install_request.clone(),
                authorization(&session_token(
                    fixture.user_id,
                    BrowserSessionSid::new(),
                    UI_AUDIENCE,
                )),
            )
            .await
            .expect_err("signed assertion without an active parent must be rejected");
        assert_eq!(inactive_parent.code, ErrorCode::Unauthenticated);
        let wrong_organization = release
            .install_ui_with_options(
                InstallUiRequest {
                    context: request_context("cross-org-target").into(),
                    organization_id: opaque(fixture.foreign_organization_id).into(),
                    ..install_request.clone()
                },
                authorization(&session_token(fixture.user_id, fixture.sid, UI_AUDIENCE)),
            )
            .await
            .expect_err("target organization mismatch must be rejected");
        assert_eq!(wrong_organization.code, ErrorCode::InvalidArgument);
        let denied_installations: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
            .fetch_one(&pool)
            .await
            .expect("count installations after denied requests");
        assert_eq!(denied_installations, 0);
        let installed = release
            .install_ui_with_options(install_request.clone(), authorization(&install_token))
            .await
            .expect("authenticated UI install")
            .into_owned();
        let receipt = installed.receipt.as_option().expect("install receipt");
        assert_receipt_scope(&pool, receipt, "project", "project").await;
        let replay = release
            .install_ui_with_options(install_request, authorization(&install_token))
            .await
            .expect("exact install replay")
            .into_owned();
        assert_eq!(replay.installation_id, installed.installation_id);
        assert_eq!(replay.generation_id, installed.generation_id);
        assert_eq!(replay.receipt, installed.receipt);

        let organization = organization_target();
        let organization_installed = release
            .install_ui_with_options(
                InstallUiRequest {
                    context: request_context("ui-install-organization").into(),
                    organization_id: opaque(fixture.organization_id).into(),
                    target: Some(organization.clone()).into(),
                    release_id: opaque(fixture.global_release_id).into(),
                    ui_key: String::from("assistant"),
                    ..Default::default()
                },
                authorization(&install_token),
            )
            .await
            .expect("authenticated organization UI install")
            .into_owned();
        assert_receipt_scope(
            &pool,
            organization_installed
                .receipt
                .as_option()
                .expect("organization receipt"),
            "organization",
            "organization",
        )
        .await;
        let repository = repository_target(fixture.repository_id);
        let repository_installed = release
            .install_ui_with_options(
                InstallUiRequest {
                    context: request_context("ui-install-repository").into(),
                    organization_id: opaque(fixture.organization_id).into(),
                    target: Some(repository.clone()).into(),
                    release_id: opaque(fixture.repository_release_id).into(),
                    ui_key: String::from("assistant"),
                    ..Default::default()
                },
                authorization(&install_token),
            )
            .await
            .expect("authenticated repository UI install")
            .into_owned();
        assert_receipt_scope(
            &pool,
            repository_installed
                .receipt
                .as_option()
                .expect("repository receipt"),
            "repository",
            "project",
        )
        .await;
        let global_console = release
            .install_ui_with_options(
                InstallUiRequest {
                    context: request_context("ui-install-global-console").into(),
                    organization_id: opaque(fixture.organization_id).into(),
                    target: Some(organization.clone()).into(),
                    release_id: opaque(fixture.global_release_id).into(),
                    ui_key: String::from("console"),
                    ..Default::default()
                },
                authorization(&install_token),
            )
            .await
            .expect("second organization UI install for cursor paging")
            .into_owned();
        assert_receipt_scope(
            &pool,
            global_console
                .receipt
                .as_option()
                .expect("global console receipt"),
            "organization",
            "organization",
        )
        .await;

        let organization_activated = release
            .activate_ui_with_options(
                ActivateUiRequest {
                    context: request_context("ui-activate-organization").into(),
                    installation_id: organization_installed.installation_id.clone(),
                    expected_generation_id: organization_installed.generation_id.clone(),
                    release_id: opaque(fixture.global_release_id).into(),
                    ui_key: String::from("assistant"),
                    ..Default::default()
                },
                authorization(&session_token(
                    fixture.user_id,
                    fixture.sid,
                    ACTIVATE_AUDIENCE,
                )),
            )
            .await
            .expect("activate organization UI")
            .into_owned();
        assert_receipt_scope(
            &pool,
            organization_activated
                .receipt
                .as_option()
                .expect("organization activation receipt"),
            "organization",
            "organization",
        )
        .await;
        let repository_activated = release
            .activate_ui_with_options(
                ActivateUiRequest {
                    context: request_context("ui-activate-repository").into(),
                    installation_id: repository_installed.installation_id.clone(),
                    expected_generation_id: repository_installed.generation_id.clone(),
                    release_id: opaque(fixture.repository_release_id).into(),
                    ui_key: String::from("assistant"),
                    ..Default::default()
                },
                authorization(&session_token(
                    fixture.user_id,
                    fixture.sid,
                    ACTIVATE_AUDIENCE,
                )),
            )
            .await
            .expect("activate repository UI")
            .into_owned();
        assert_receipt_scope(
            &pool,
            repository_activated
                .receipt
                .as_option()
                .expect("repository activation receipt"),
            "repository",
            "project",
        )
        .await;

        let list_token = session_token(fixture.user_id, fixture.sid, LIST_AUDIENCE);
        let list_request = ListUiInstallationsRequest {
            organization_id: opaque(fixture.organization_id).into(),
            target: Some(target.clone()).into(),
            page: PageRequest {
                page_size: 50,
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        let listed = release
            .list_ui_installations_with_options(list_request.clone(), authorization(&list_token))
            .await
            .expect("list installed UI")
            .into_owned();
        assert_eq!(listed.installations.len(), 1);
        assert!(listed.installations[0].launchable);
        assert!(listed.page.as_option().is_some());
        let tampered_cursor = release
            .list_ui_installations_with_options(
                ListUiInstallationsRequest {
                    page: PageRequest {
                        page_token: String::from("tampered-cursor"),
                        ..Default::default()
                    }
                    .into(),
                    ..list_request
                },
                authorization(&list_token),
            )
            .await
            .expect_err("tampered installation cursor must be rejected");
        assert_eq!(tampered_cursor.code, ErrorCode::InvalidArgument);

        let organization_list_request = ListUiInstallationsRequest {
            organization_id: opaque(fixture.organization_id).into(),
            target: Some(organization.clone()).into(),
            page: PageRequest {
                page_size: 1,
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        let organization_page = release
            .list_ui_installations_with_options(
                organization_list_request.clone(),
                authorization(&list_token),
            )
            .await
            .expect("list organization UI installations")
            .into_owned();
        assert_eq!(organization_page.installations.len(), 1);
        let scoped_cursor = organization_page
            .page
            .as_option()
            .expect("organization page response")
            .next_page_token
            .clone();
        assert!(
            !scoped_cursor.is_empty(),
            "organization list must produce a cursor"
        );

        let second_actor_cursor = release
            .list_ui_installations_with_options(
                ListUiInstallationsRequest {
                    page: PageRequest {
                        page_token: scoped_cursor.clone(),
                        ..Default::default()
                    }
                    .into(),
                    ..organization_list_request.clone()
                },
                authorization(&session_token(
                    fixture.second_user_id,
                    fixture.second_sid,
                    LIST_AUDIENCE,
                )),
            )
            .await
            .expect_err("cursor must reject a different actor");
        assert_eq!(second_actor_cursor.code, ErrorCode::InvalidArgument);

        let second_organization_cursor = release
            .list_ui_installations_with_options(
                ListUiInstallationsRequest {
                    organization_id: opaque(fixture.foreign_organization_id).into(),
                    target: Some(organization_target()).into(),
                    page: PageRequest {
                        page_token: scoped_cursor.clone(),
                        ..Default::default()
                    }
                    .into(),
                    ..Default::default()
                },
                authorization(&list_token),
            )
            .await
            .expect_err("cursor must reject a different organization");
        assert_eq!(second_organization_cursor.code, ErrorCode::InvalidArgument);

        let second_target_cursor = release
            .list_ui_installations_with_options(
                ListUiInstallationsRequest {
                    target: Some(target.clone()).into(),
                    page: PageRequest {
                        page_token: scoped_cursor,
                        ..Default::default()
                    }
                    .into(),
                    ..organization_list_request
                },
                authorization(&list_token),
            )
            .await
            .expect_err("cursor must reject a different target");
        assert_eq!(second_target_cursor.code, ErrorCode::InvalidArgument);

        let handoff_token = session_token(fixture.user_id, fixture.sid, HANDOFF_AUDIENCE);
        let secret = vec![0x5a; 32];
        let handoffs_before_invalid_secret: i64 =
            sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
                .fetch_one(&pool)
                .await
                .expect("count handoffs before invalid secret");
        let audit_before_invalid_secret: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM ui_request_audit_events
              WHERE surface = 'handoff_issue'",
        )
        .fetch_one(&pool)
        .await
        .expect("count handoff audit rows before invalid secret");
        let invalid_secret_request_id = Uuid::new_v4();
        let invalid_secret = release
            .create_ui_browser_handoff_with_options(
                CreateUiBrowserHandoffRequest {
                    context: RequestContext {
                        request_id: opaque(invalid_secret_request_id).into(),
                        ..Default::default()
                    }
                    .into(),
                    installation_id: installed.installation_id.clone(),
                    generation_id: installed.generation_id.clone(),
                    route: String::from("assistant"),
                    handoff_secret: vec![0x5a; 31],
                    ..Default::default()
                },
                authorization(&handoff_token),
            )
            .await
            .expect_err("invalid handoff secret must be rejected by production handler");
        assert_eq!(invalid_secret.code, ErrorCode::InvalidArgument);
        let handoffs_after_invalid_secret: i64 =
            sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
                .fetch_one(&pool)
                .await
                .expect("count handoffs after invalid secret");
        assert_eq!(
            handoffs_after_invalid_secret, handoffs_before_invalid_secret,
            "invalid secret must not create a handoff row"
        );
        let audit_after_invalid_secret: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM ui_request_audit_events
              WHERE surface = 'handoff_issue'",
        )
        .fetch_one(&pool)
        .await
        .expect("count handoff audit rows after invalid secret");
        assert_eq!(
            audit_after_invalid_secret,
            audit_before_invalid_secret + 1,
            "invalid secret must create exactly one handoff denial audit"
        );
        let invalid_secret_audit: InvalidSecretAudit = sqlx::query_as(
            "SELECT request_id, decision, outcome, reason_code,
                    actor_id, organization_id, installation_id, generation_id
               FROM ui_request_audit_events
              WHERE surface = 'handoff_issue'
              ORDER BY occurred_at DESC, id DESC
              LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load invalid secret audit row");
        assert_eq!(invalid_secret_audit.1, "denied");
        assert_eq!(invalid_secret_audit.2, "not_attempted");
        assert_eq!(invalid_secret_audit.3, "invalid_input");
        assert_eq!(invalid_secret_audit.4, Some(fixture.user_id));
        assert_eq!(invalid_secret_audit.5, None);
        assert_eq!(invalid_secret_audit.6, None);
        assert_eq!(invalid_secret_audit.7, None);
        assert_ne!(invalid_secret_audit.0, Uuid::nil());
        assert_ne!(
            invalid_secret_audit.0, invalid_secret_request_id,
            "audit correlation must use the server marker, not caller input"
        );

        let handoffs_before_malformed_wire: i64 =
            sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
                .fetch_one(&pool)
                .await
                .expect("count handoffs before malformed wire request");
        let audit_before_malformed_wire: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM ui_request_audit_events
              WHERE surface = 'handoff_issue'",
        )
        .fetch_one(&pool)
        .await
        .expect("count handoff audits before malformed wire request");
        let malformed_wire = reqwest::Client::new()
            .post(format!(
                "http://{}/hephaestus.release.v1.ReleaseService/CreateUiBrowserHandoff",
                app.http_addr()
            ))
            .header("authorization", format!("Bearer {handoff_token}"))
            .header("content-type", "application/proto")
            .body(vec![0xff, 0x00, 0x7f])
            .send()
            .await
            .expect("send malformed handoff wire request");
        let malformed_wire_status = malformed_wire.status();
        let _ = malformed_wire
            .bytes()
            .await
            .expect("read malformed handoff wire response");
        assert!(
            malformed_wire_status.is_client_error(),
            "malformed wire request must retain a client error status"
        );
        let handoffs_after_malformed_wire: i64 =
            sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
                .fetch_one(&pool)
                .await
                .expect("count handoffs after malformed wire request");
        assert_eq!(
            handoffs_after_malformed_wire, handoffs_before_malformed_wire,
            "malformed wire request must not create a handoff row"
        );
        let audit_after_malformed_wire: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM ui_request_audit_events
              WHERE surface = 'handoff_issue'",
        )
        .fetch_one(&pool)
        .await
        .expect("count handoff audits after malformed wire request");
        assert_eq!(
            audit_after_malformed_wire,
            audit_before_malformed_wire + 1,
            "malformed wire request must create exactly one transport denial audit"
        );
        let malformed_wire_audit: (String, String, String, Option<Uuid>) = sqlx::query_as(
            "SELECT decision, outcome, reason_code, actor_id
               FROM ui_request_audit_events
              WHERE surface = 'handoff_issue'
              ORDER BY occurred_at DESC, id DESC
              LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load malformed wire audit row");
        assert_eq!(malformed_wire_audit.0, "denied");
        assert_eq!(malformed_wire_audit.1, "not_attempted");
        assert_eq!(malformed_wire_audit.2, "invalid_input");
        assert_eq!(malformed_wire_audit.3, Some(fixture.user_id));
        let valid_handoff_request_id = Uuid::new_v4();
        let handoff = release
            .create_ui_browser_handoff_with_options(
                CreateUiBrowserHandoffRequest {
                    context: RequestContext {
                        request_id: opaque(valid_handoff_request_id).into(),
                        ..Default::default()
                    }
                    .into(),
                    installation_id: installed.installation_id.clone(),
                    generation_id: installed.generation_id.clone(),
                    route: String::from("assistant"),
                    handoff_secret: secret.clone(),
                    ..Default::default()
                },
                authorization(&handoff_token),
            )
            .await
            .expect("authenticated UI handoff")
            .into_owned();
        assert!(handoff.handoff_id.as_option().is_some());
        assert_eq!(handoff.route, "assistant");
        let handoff_id =
            Uuid::parse_str(&handoff.handoff_id.as_option().expect("handoff ID").value)
                .expect("canonical handoff ID");
        let stored_handoff_request_id: Uuid =
            sqlx::query_scalar("SELECT request_id FROM ui_browser_handoffs WHERE id = $1")
                .bind(handoff_id)
                .fetch_one(&pool)
                .await
                .expect("load handoff correlation ID");
        assert_ne!(
            stored_handoff_request_id, valid_handoff_request_id,
            "worker store must receive the server marker, not caller input"
        );
        let handoff_debug = format!("{handoff:?}");
        assert!(!handoff_debug.contains("5a5a5a"));
        assert!(!handoff_debug.contains(&fixture.parent_session_id.to_string()));

        let denied = release
            .create_ui_browser_handoff_with_options(
                CreateUiBrowserHandoffRequest {
                    context: request_context("must-be-empty").into(),
                    installation_id: installed.installation_id.clone(),
                    generation_id: installed.generation_id.clone(),
                    route: String::from("assistant"),
                    handoff_secret: secret,
                    ..Default::default()
                },
                authorization(&handoff_token),
            )
            .await
            .expect_err("handoff idempotency key must be rejected");
        assert_eq!(denied.code, ErrorCode::InvalidArgument);

        let activated = release
            .activate_ui_with_options(
                ActivateUiRequest {
                    context: request_context("ui-activate").into(),
                    installation_id: installed.installation_id.clone(),
                    expected_generation_id: installed.generation_id.clone(),
                    release_id: opaque(fixture.release_id).into(),
                    ui_key: String::from("assistant"),
                    ..Default::default()
                },
                authorization(&session_token(
                    fixture.user_id,
                    fixture.sid,
                    ACTIVATE_AUDIENCE,
                )),
            )
            .await
            .expect("activate UI with current CAS")
            .into_owned();
        assert_ne!(activated.generation_id, installed.generation_id);

        let rolled_back = release
            .rollback_ui_with_options(
                RollbackUiRequest {
                    context: request_context("ui-rollback").into(),
                    installation_id: installed.installation_id.clone(),
                    expected_generation_id: activated.generation_id.clone(),
                    release_id: opaque(fixture.release_id).into(),
                    ui_key: String::from("assistant"),
                    ..Default::default()
                },
                authorization(&session_token(
                    fixture.user_id,
                    fixture.sid,
                    ROLLBACK_AUDIENCE,
                )),
            )
            .await
            .expect("rollback UI with current CAS")
            .into_owned();
        assert_ne!(rolled_back.generation_id, activated.generation_id);

        let disabled = release
            .disable_ui_with_options(
                DisableUiRequest {
                    context: request_context("ui-disable").into(),
                    installation_id: installed.installation_id.clone(),
                    expected_generation_id: rolled_back.generation_id.clone(),
                    ..Default::default()
                },
                authorization(&session_token(
                    fixture.user_id,
                    fixture.sid,
                    DISABLE_AUDIENCE,
                )),
            )
            .await
            .expect("disable UI with current CAS")
            .into_owned();
        assert_eq!(disabled.lifecycle.to_i32(), 2);
        assert_eq!(disabled.generation_id, rolled_back.generation_id);

        let disabled_installation_id = disabled
            .installation_id
            .as_option()
            .expect("disabled installation id")
            .value
            .parse::<Uuid>()
            .expect("disabled installation UUID");
        let commands_before_stale: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1",
        )
        .bind(disabled_installation_id)
        .fetch_one(&pool)
        .await
        .expect("count lifecycle commands before stale CAS");

        let stale = release
            .remove_ui_with_options(
                RemoveUiRequest {
                    context: request_context("ui-stale-remove").into(),
                    installation_id: disabled.installation_id.clone(),
                    expected_generation_id: installed.generation_id,
                    ..Default::default()
                },
                authorization(&session_token(
                    fixture.user_id,
                    fixture.sid,
                    REMOVE_AUDIENCE,
                )),
            )
            .await
            .expect_err("removed lifecycle must not accept stale mutation");
        assert_eq!(stale.code, ErrorCode::FailedPrecondition);
        let commands_after_stale: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1",
        )
        .bind(disabled_installation_id)
        .fetch_one(&pool)
        .await
        .expect("count lifecycle commands after stale CAS");
        assert_eq!(commands_after_stale, commands_before_stale);
        let removed = release
            .remove_ui_with_options(
                RemoveUiRequest {
                    context: request_context("ui-remove").into(),
                    installation_id: disabled.installation_id,
                    expected_generation_id: disabled.generation_id,
                    ..Default::default()
                },
                authorization(&session_token(
                    fixture.user_id,
                    fixture.sid,
                    REMOVE_AUDIENCE,
                )),
            )
            .await
            .expect("remove UI with current CAS")
            .into_owned();
        assert_eq!(removed.lifecycle.to_i32(), 3);
        sqlx::query(
            "UPDATE human_browser_sessions
             SET revoked_at = now(), revocation_reason = 'logout'
             WHERE id = $1",
        )
        .bind(fixture.parent_session_id)
        .execute(&pool)
        .await
        .expect("revoke parent browser session");
        let revoked_parent = release
            .list_ui_installations_with_options(
                ListUiInstallationsRequest {
                    organization_id: opaque(fixture.organization_id).into(),
                    target: Some(target).into(),
                    ..Default::default()
                },
                authorization(&list_token),
            )
            .await
            .expect_err("revoked parent session must be rejected");
        assert_eq!(revoked_parent.code, ErrorCode::Unauthenticated);
        println!(
            "REAL_UI_INSTALLATION_RPC=1 install_replay=1 organization=1 repository=1 receipts=1 list=1 cursor_actor=1 cursor_org=1 cursor_target=1 cursor_tamper=1 handoff_secret=32 handoff_parent_redacted=1 lifecycle_all5=1 stale_cas=1 wrong_tenant=1 wrong_audience=1 expired=1 revoked=1"
        );
        app.shutdown().await.expect("shutdown UI RPC application");
        pool.close().await;
        cleanup_nats(&nats_url).await;
    }
}
