#![cfg(feature = "test-fixtures")]

//! Real Connect transport evidence for artifact request deadlines.

#[path = "../support/browser_session_fixture.rs"]
mod app_fixture;

#[cfg(test)]
mod artifact_deadline_transport {
    #[path = "artifact_deadline_stream.rs"]
    mod stream_support;

    use self::stream_support::request_context;
    #[path = "artifact_deadline_fixture.rs"]
    mod fixture_support;
    use super::app_fixture::{app_config, cleanup_nats, require_disposable_nats};
    use connectrpc::{
        Protocol,
        client::{CallOptions, ClientConfig, Http2Connection, ServiceTransport},
        error::ErrorCode,
    };
    use fixture_support::{bootstrap_token, opaque, seed_artifact, session_token};
    use hephaestus_app::HephaestusApp;
    use identity_domain::BrowserSessionSid;
    use rpc_proto::{
        connect::hephaestus::{
            artifact::v1::ArtifactServiceClient, identity::v1::IdentityServiceClient,
        },
        messages::hephaestus::identity::v1::CreateBrowserSessionRequest,
    };
    use sqlx::postgres::PgPoolOptions;
    use std::{env, time::Duration};
    use tempfile::tempdir;
    use uuid::Uuid;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[serial_test::serial]
    // The daemon spans deadline/cancellation probes and is consumed by awaited shutdown.
    #[allow(clippy::significant_drop_tightening)]
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
        let app = Box::pin(HephaestusApp::build(app_config(
            database_url,
            nats_url.clone(),
            root.path(),
        )))
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
