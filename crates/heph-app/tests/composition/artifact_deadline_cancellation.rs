#![cfg(feature = "test-fixtures")]

//! Real Connect transport evidence for artifact request deadlines.

#[path = "../support/browser_session_fixture.rs"]
mod app_fixture;

#[cfg(test)]
mod artifact_deadline_transport {
    #[path = "artifact_deadline_stream.rs"]
    mod stream_support;

    use self::stream_support::{now_seconds, request_context};
    use super::app_fixture::{app_config, cleanup_nats, require_disposable_nats};
    use connectrpc::{
        Protocol,
        client::{CallOptions, ClientConfig, Http2Connection, ServiceTransport},
        error::ErrorCode,
    };
    use hephaestus_app::HephaestusApp;
    use identity_domain::BrowserSessionSid;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use rpc_proto::{
        connect::hephaestus::{
            artifact::v1::ArtifactServiceClient, identity::v1::IdentityServiceClient,
        },
        messages::hephaestus::common::v1::OpaqueId,
        messages::hephaestus::identity::v1::CreateBrowserSessionRequest,
    };
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use sqlx::{PgPool, postgres::PgPoolOptions};
    use std::{env, fs, path::Path, time::Duration};
    use tempfile::tempdir;
    use uuid::Uuid;

    const SECRET: &[u8] = b"service-log-retention-test-signing-secret";
    const ISSUER: &str = "https://artifact-rpc-test.invalid";
    const CONTENT: &[u8] = b"deadline artifact fixture\n";
    const AUDIENCE: &str = "/hephaestus.artifact.v1.ArtifactService/StreamArtifact";
    const CREATE_AUDIENCE: &str = "/hephaestus.identity.v1.IdentityService/CreateBrowserSession";

    fn opaque(id: Uuid) -> OpaqueId {
        OpaqueId {
            value: id.to_string(),
            ..Default::default()
        }
    }

    fn session_token(user_id: Uuid, sid: BrowserSessionSid) -> String {
        let now = now_seconds();
        encode(
            &Header::new(Algorithm::HS256),
            &json!({
                "iss": "hephaestus-web-mediator",
                "sub": user_id.to_string(),
                "aud": AUDIENCE,
                "iat": now,
                "nbf": now,
                "exp": now + 25,
                "jti": Uuid::new_v4().to_string(),
                "sid": sid.to_protocol_string(),
            }),
            &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(SECRET)),
        )
        .expect("sign artifact session assertion")
    }

    fn bootstrap_token(issuer: &str, subject: &str) -> String {
        let now = now_seconds();
        encode(
            &Header::new(Algorithm::HS256),
            &json!({
                "iss": "hephaestus-web-mediator",
                "sub": "hephaestus-web-mediator",
                "aud": CREATE_AUDIENCE,
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
        .expect("sign artifact browser-session bootstrap assertion")
    }

    // Keep the relational fixture together so the integration test exercises the
    // same artifact records and storage path that the production adapter reads.
    #[allow(clippy::too_many_lines)]
    async fn seed_artifact(pool: &PgPool, root: &Path) -> (Uuid, Uuid, String, String) {
        let user = Uuid::new_v4();
        let organization = Uuid::new_v4();
        let project = Uuid::new_v4();
        let repository = Uuid::new_v4();
        let build = Uuid::new_v4();
        let release = Uuid::new_v4();
        let artifact = Uuid::new_v4();
        let storage_key = Uuid::new_v4();
        let issuer = format!("{ISSUER}/{user}");
        let subject = format!("subject-{user}");
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(user)
            .bind(format!("artifact deadline user {user}"))
            .execute(pool)
            .await
            .expect("seed artifact user");
        sqlx::query("INSERT INTO external_identities (user_id, issuer, subject, provider_metadata) VALUES ($1, $2, $3, '{}'::jsonb)")
            .bind(user)
            .bind(&issuer)
            .bind(&subject)
            .execute(pool)
            .await
            .expect("seed artifact external identity");
        sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
            .bind(organization)
            .bind(format!("artifact deadline org {user}"))
            .execute(pool)
            .await
            .expect("seed artifact organization");
        sqlx::query(
            "INSERT INTO organization_members (organization_id, user_id, role)
             VALUES ($1, $2, 'owner')",
        )
        .bind(organization)
        .bind(user)
        .execute(pool)
        .await
        .expect("seed artifact membership");
        sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
            .bind(project)
            .bind(organization)
            .bind(format!("deadline project {user}"))
            .execute(pool)
            .await
            .expect("seed artifact project");
        sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
            .bind(repository)
            .bind(project)
            .bind(format!("deadline repository {user}"))
            .execute(pool)
            .await
            .expect("seed artifact repository");
        sqlx::query(
            "INSERT INTO build_requests
             (id, repository_id, source_commit, source_ref, build_definition_hash, state, created_by)
             VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
        )
        .bind(build)
        .bind(repository)
        .bind("a".repeat(40))
        .bind([1_u8; 32].as_slice())
        .bind(user)
        .execute(pool)
        .await
        .expect("seed artifact build");
        sqlx::query(
            "INSERT INTO releases
             (id, repository_id, version, source_commit, source_ref, build_request_id,
              build_definition_hash, configuration, configuration_hash, manifest_hash,
              state, publication_actor_id, published_at)
             VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5, '{}', $6, $7,
                     'published', $8, now())",
        )
        .bind(release)
        .bind(repository)
        .bind("a".repeat(40))
        .bind(build)
        .bind([1_u8; 32].as_slice())
        .bind([2_u8; 32].as_slice())
        .bind([3_u8; 32].as_slice())
        .bind(user)
        .execute(pool)
        .await
        .expect("seed artifact release");
        let content_hash: [u8; 32] = Sha256::digest(CONTENT).into();
        sqlx::query(
            "INSERT INTO release_artifacts
             (id, release_id, path, kind, mode, content_hash, size_bytes, media_type, storage_key)
             VALUES ($1, $2, 'deadline.txt', 'file', 292, $3, $4, 'text/plain', $5)",
        )
        .bind(artifact)
        .bind(release)
        .bind(content_hash.as_slice())
        .bind(i64::try_from(CONTENT.len()).expect("content length"))
        .bind(storage_key)
        .execute(pool)
        .await
        .expect("seed artifact metadata");
        let artifact_root = root.join("release-artifacts");
        fs::create_dir_all(&artifact_root).expect("create artifact root");
        fs::write(
            artifact_root.join(storage_key.simple().to_string()),
            CONTENT,
        )
        .expect("write artifact content");
        (artifact, user, issuer, subject)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[serial_test::serial]
    async fn artifact_rpc_propagates_transport_deadline_to_real_adapter() {
        if env::var("REAL_APP_ARTIFACT_RPC").as_deref() != Ok("1") {
            eprintln!("SKIPPED artifact RPC deadline evidence: set REAL_APP_ARTIFACT_RPC=1");
            return;
        }
        let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("test database URL");
        let nats_url = env::var("HEPHAESTUS_NATS_TEST_URL").expect("test NATS URL");
        require_disposable_nats(&nats_url);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&database_url)
            .await
            .expect("connect artifact RPC database");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("apply artifact RPC migrations");
        let root = tempdir().expect("artifact RPC root");
        let (artifact, user, issuer, subject) = seed_artifact(&pool, root.path()).await;
        let app = HephaestusApp::build(app_config(database_url, nats_url.clone(), root.path()))
            .await
            .expect("build artifact RPC application")
            .start()
            .await
            .expect("start artifact RPC application");
        let uri: axum::http::Uri = format!("http://{}", app.http_addr())
            .parse()
            .expect("artifact RPC URI");
        let raw_connection = Http2Connection::connect_plaintext(uri.clone())
            .await
            .expect("connect artifact RPC");
        let (buffer, worker) = tower::buffer::Buffer::pair(raw_connection, 2);
        let connection_worker = tokio::spawn(async move {
            worker.await;
        });
        let connection = ServiceTransport::new(
            tower::ServiceBuilder::new()
                .map_err(stream_support::TransportError::from_display)
                .service(buffer),
        );
        let sid = BrowserSessionSid::new();
        let identity = IdentityServiceClient::new(
            connection.clone(),
            ClientConfig::new(uri.clone()).with_protocol(Protocol::Connect),
        );
        identity
            .create_browser_session_with_options(
                CreateBrowserSessionRequest {
                    context: request_context("artifact-deadline-session").into(),
                    issuer: issuer.clone(),
                    subject: subject.clone(),
                    sid: sid
                        .to_protocol_string()
                        .parse::<Uuid>()
                        .expect("session SID UUID")
                        .into_bytes()
                        .to_vec(),
                    ..Default::default()
                },
                CallOptions::default().with_header(
                    "authorization",
                    format!("Bearer {}", bootstrap_token(&issuer, &subject)),
                ),
            )
            .await
            .expect("create artifact browser session");
        let client = ArtifactServiceClient::new(
            connection.clone(),
            ClientConfig::new(uri.clone()).with_protocol(Protocol::Connect),
        );
        let error =
            stream_support::artifact_stream_error(client.clone(), artifact, user, sid, "1").await;
        assert_eq!(error.code, ErrorCode::DeadlineExceeded);

        let mut lock_transaction = pool
            .begin()
            .await
            .expect("begin artifact authorization lock transaction");
        sqlx::query("LOCK TABLE release_artifacts IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *lock_transaction)
            .await
            .expect("lock artifact metadata table");
        let locked_client = client.clone();
        let mut rpc = Box::pin(async move {
            stream_support::artifact_stream_error(locked_client, artifact, user, sid, "500").await
        });
        tokio::select! {
            result = &mut rpc => panic!("artifact RPC completed before reaching its lock: {result:?}"),
            () = stream_support::wait_for_artifact_lock(&pool) => {}
        }
        let waiting = tokio::time::timeout(Duration::from_secs(2), &mut rpc)
            .await
            .expect("locked artifact RPC must remain bounded");
        assert_eq!(waiting.code, ErrorCode::DeadlineExceeded);
        stream_support::wait_for_artifact_query_to_terminate(&pool).await;
        lock_transaction
            .rollback()
            .await
            .expect("release artifact authorization lock");

        let mut cancellation_lock = pool
            .begin()
            .await
            .expect("begin artifact cancellation lock transaction");
        sqlx::query("LOCK TABLE release_artifacts IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *cancellation_lock)
            .await
            .expect("lock artifact metadata table for cancellation");
        let mut raw_request =
            stream_support::start_raw_artifact_request(uri.clone(), artifact, user, sid).await;
        raw_request.await_headers().await;
        stream_support::wait_for_artifact_lock(&pool).await;
        raw_request.reset();
        stream_support::wait_for_artifact_query_to_terminate(&pool).await;
        raw_request.finish().await;
        drop(client);
        drop(identity);
        drop(connection);
        if !connection_worker.is_finished() {
            connection_worker.abort();
        }
        if let Err(error) = connection_worker.await {
            assert!(
                error.is_cancelled(),
                "artifact connection worker failed: {error}"
            );
        }
        let pool_probe: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&pool)
            .await
            .expect("artifact pool remains usable after cancellation");
        assert_eq!(pool_probe, 1);
        cancellation_lock
            .rollback()
            .await
            .expect("release artifact cancellation lock");
        app.shutdown()
            .await
            .expect("shutdown artifact RPC application");
        cleanup_nats(&nats_url).await;
        println!("REAL_APP_ARTIFACT_RPC_COMPLETED=1");
    }
}
