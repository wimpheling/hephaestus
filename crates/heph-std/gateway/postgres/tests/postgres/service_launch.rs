/// service launch scenario.
use super::support::{RecordingServiceMaterializer, seed_fixture};
use gateway_domain::{
    GatewayServiceIdentity, GatewayServiceLaunchRequest, GatewayServiceLaunchResolver,
    GatewayServiceMaterializer,
};
use gateway_postgres::PostgresGatewayServiceLaunchResolver;
use serial_test::serial;
use sha2::Digest;
use sha2::Sha256;
use sqlx::postgres::PgPoolOptions;
use std::{collections::BTreeMap, env, path::PathBuf, sync::Arc, time::Duration};
use uuid::Uuid;
use vm_trait::{NetworkMode, RootFilesystem};

#[tokio::test]
#[serial]
async fn gateway_service_launch_resolves_exact_published_revision_without_runtime_session() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("SKIPPED gateway service resolver: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect gateway service resolver PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway service resolver migrations");
    let max_migration: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read migration marker");
    assert!(max_migration >= 77, "migration 0077 must be applied");
    println!(
        "REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_migration} service_launch_resolver=connected"
    );
    let fixture = seed_fixture(&pool).await;
    println!("REAL_POSTGRES_SERVICE_RESOLVER_FIXTURE=seeded");
    let service_revision = Uuid::new_v4();
    let service_agent = Uuid::new_v4();
    let artifact_key = Uuid::new_v4();
    let artifact_hash = Sha256::digest(b"service executable");
    let runtime_contract = serde_json::json!({
        "command": "bin/server",
        "arguments": ["--serve"],
        "working_directory": ".",
        "image_reference": "service-root",
        "requires_state": false,
        "policy_ceiling": {"vcpus": 2, "memory_mib": 256, "network": "disabled"}
    });
    sqlx::query("INSERT INTO release_agents (id, release_id, family_id, agent_key, display_name, runtime_contract, runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state) VALUES ($1, $2, $3, 'service-agent', 'Service agent', $4, $5, '[]', '[]', false)")
        .bind(service_agent)
        .bind(fixture.release)
        .bind(fixture.family)
        .bind(runtime_contract)
        .bind([7_u8; 32].as_slice())
        .execute(&pool)
        .await
        .expect("service release agent");
    sqlx::query("INSERT INTO gateway_revisions (id, gateway_id, project_id, repository_id, release_id, release_agent_id, release_agent_key, handler_contract, exposure, parameters, service_loopback_port, service_readiness_path, service_health_path, service_log_capture_mode, normalized_hash, created_by) VALUES ($1, $2, $3, $4, $5, $6, 'service-agent', 'http.service.v1', 'public', $7, 18081, '/ready', '/health', 'application', $8, $9)")
        .bind(service_revision)
        .bind(fixture.gateway)
        .bind(fixture.project)
        .bind(fixture.repository)
        .bind(fixture.release)
        .bind(service_agent)
        .bind(serde_json::json!({"mode": "persistent"}))
        .bind([8_u8; 32].as_slice())
        .bind(fixture.owner)
        .execute(&pool)
        .await
        .expect("service gateway revision");
    sqlx::query("INSERT INTO release_artifacts (id, release_id, path, kind, mode, content_hash, size_bytes, media_type, storage_key) VALUES ($1, $2, 'bin/server', 'executable', 365, $3, 17, 'application/octet-stream', $4)")
        .bind(Uuid::new_v4())
        .bind(fixture.release)
        .bind(artifact_hash.as_slice())
        .bind(artifact_key)
        .execute(&pool)
        .await
        .expect("service release artifact");

    let before: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM gateway_invocations),
                (SELECT count(*) FROM gateway_runtime_authority_sessions)",
    )
    .fetch_one(&pool)
    .await
    .expect("count runtime rows before resolution");
    let worker_pool = PgPoolOptions::new()
        .max_connections(6)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect service resolver worker role");
    let materializer = Arc::new(RecordingServiceMaterializer::default());
    let resolver = PostgresGatewayServiceLaunchResolver::new(
        worker_pool,
        BTreeMap::from([(
            String::from("service-root"),
            RootFilesystem::Directory {
                host_path: PathBuf::from("/var/lib/hephaestus/test-root"),
            },
        )]),
    )
    .with_service_materializer(Arc::clone(&materializer) as Arc<dyn GatewayServiceMaterializer>);
    let identity = GatewayServiceIdentity {
        instance_id: Uuid::new_v4(),
        gateway_id: fixture.gateway,
        revision_id: service_revision,
    };
    let launch = resolver
        .resolve_service_launch(GatewayServiceLaunchRequest { identity })
        .await
        .expect("resolve published service revision");
    println!("REAL_POSTGRES_SERVICE_RESOLVER_QUERY=passed");
    assert_eq!(launch.identity, identity);
    assert_eq!(launch.service.loopback_port, 18081);
    assert_eq!(launch.service.readiness_path.as_str(), "/ready");
    assert_eq!(launch.service.health_path.as_str(), "/health");
    assert_eq!(
        launch.service.log_capture_mode,
        gateway_domain::ServiceLogCaptureMode::Application
    );
    assert!(matches!(launch.spec.network, NetworkMode::Disabled));
    assert!(launch.spec.runtime_authority.is_none());
    assert_eq!(
        launch
            .spec
            .private_http_service
            .as_ref()
            .unwrap()
            .max_connections,
        32
    );
    assert_eq!(
        launch
            .spec
            .private_http_service
            .as_ref()
            .unwrap()
            .connect_timeout,
        Duration::from_secs(2)
    );
    assert_eq!(
        launch.spec.id.0,
        format!("gateway-service-{}", identity.instance_id)
    );
    assert_eq!(
        launch.spec.labels.get("hephaestus.gateway-instance"),
        Some(&identity.instance_id.to_string())
    );
    {
        let recorded = materializer.records.lock().expect("materializer records");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].identity, identity);
        assert_eq!(recorded[0].artifact_paths, vec![String::from("bin/server")]);
        assert_eq!(
            recorded[0].parameters,
            serde_json::json!({"mode": "persistent"})
        );
        drop(recorded);
    }
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM gateway_invocations),
                (SELECT count(*) FROM gateway_runtime_authority_sessions)",
    )
    .fetch_one(&pool)
    .await
    .expect("count runtime rows after resolution");
    assert_eq!(
        after, before,
        "service resolution must not create runtime rows"
    );

    let wrong_revision = resolver
        .resolve_service_launch(GatewayServiceLaunchRequest {
            identity: GatewayServiceIdentity {
                revision_id: fixture.revision,
                ..identity
            },
        })
        .await;
    assert!(
        wrong_revision.is_err(),
        "stateless revisions cannot be service launches"
    );
    let wrong_gateway = resolver
        .resolve_service_launch(GatewayServiceLaunchRequest {
            identity: GatewayServiceIdentity {
                gateway_id: Uuid::new_v4(),
                ..identity
            },
        })
        .await;
    assert!(
        wrong_gateway.is_err(),
        "gateway identity must bind the exact revision"
    );

    let invalid_materializer = Arc::new(RecordingServiceMaterializer {
        invalid_mounts: true,
        ..RecordingServiceMaterializer::default()
    });
    let invalid_resolver = PostgresGatewayServiceLaunchResolver::new(
        pool.clone(),
        BTreeMap::from([(
            String::from("service-root"),
            RootFilesystem::Directory {
                host_path: PathBuf::from("/var/lib/hephaestus/test-root"),
            },
        )]),
    )
    .with_service_materializer(
        Arc::clone(&invalid_materializer) as Arc<dyn GatewayServiceMaterializer>
    );
    assert!(
        invalid_resolver
            .resolve_service_launch(GatewayServiceLaunchRequest { identity })
            .await
            .is_err()
    );
    assert_eq!(
        invalid_materializer
            .destroyed
            .lock()
            .expect("destroyed identities")
            .as_slice(),
        &[identity]
    );

    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(fixture.release)
        .execute(&pool)
        .await
        .expect("revoke service release");
    assert!(
        resolver
            .resolve_service_launch(GatewayServiceLaunchRequest { identity })
            .await
            .is_err()
    );
}
