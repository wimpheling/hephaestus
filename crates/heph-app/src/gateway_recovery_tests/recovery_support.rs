use super::*;

pub(super) async fn wait_for_ready(pool: &sqlx::PgPool, fixture: Fixture) -> bool {
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state
                   FROM gateway_service_instances
                  WHERE gateway_id = $1 AND revision_id = $2
                  ORDER BY fencing_token DESC
                  LIMIT 1",
            )
            .bind(fixture.gateway)
            .bind(fixture.revision)
            .fetch_optional(pool)
            .await
            .expect("read automatic startup state");
            if state.as_deref() == Some("ready") {
                return true;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or(false)
}

pub(super) async fn wait_for_ready_and_active(pool: &sqlx::PgPool, fixture: Fixture) -> bool {
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let ready_and_active: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1
                      FROM gateway_service_instances AS i
                      JOIN gateways AS g ON g.id = i.gateway_id
                     WHERE i.gateway_id = $1
                       AND i.revision_id = $2
                       AND i.state = 'ready'
                       AND g.active_revision_id = $2
                )",
            )
            .bind(fixture.gateway)
            .bind(fixture.revision)
            .fetch_one(pool)
            .await
            .expect("read restored service state and active revision");
            if ready_and_active {
                return true;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or(false)
}

pub(super) async fn cleanup_startup_fixture(pool: &sqlx::PgPool, fixture: Fixture) {
    sqlx::query(
        "UPDATE gateways
            SET lifecycle = 'paused', active_revision_id = NULL,
                desired_service_revision_id = NULL
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .execute(pool)
    .await
    .expect("retire automatic startup fixture");
    sqlx::query(
        "DELETE FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .execute(pool)
    .await
    .expect("remove automatic startup inventory");
}

pub(super) async fn clear_service_log_fixture(pool: &sqlx::PgPool, fixture: Fixture) {
    sqlx::query(
        "DELETE FROM gateway_service_log_chunks
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .execute(pool)
    .await
    .expect("clear service log chunks");
    sqlx::query(
        "DELETE FROM gateway_service_log_epochs
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .execute(pool)
    .await
    .expect("clear service log epochs");
    sqlx::query(
        "DELETE FROM gateway_service_log_project_usage
          WHERE project_id = (SELECT project_id FROM gateways WHERE id = $1)",
    )
    .bind(fixture.gateway)
    .execute(pool)
    .await
    .expect("clear service log project usage");
}

pub(super) const fn make_authority(pool: sqlx::PgPool) -> PostgresGatewayEdgeAuthority {
    PostgresGatewayEdgeAuthority::new(
        pool,
        GatewayLimits {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 256,
            execution_timeout: StdDuration::from_secs(10),
        },
    )
}

pub(super) fn test_supervisor_context(pool: sqlx::PgPool) -> GatewayServiceSupervisorContext {
    test_supervisor_context_with_destroy_counter(pool).0
}

pub(super) fn test_supervisor_context_with_destroy_counter(
    pool: sqlx::PgPool,
) -> (GatewayServiceSupervisorContext, Arc<AtomicUsize>) {
    let destroyed = Arc::new(AtomicUsize::new(0));
    let context = GatewayServiceSupervisorContext {
        owner: GatewayServiceOwner::new("recovery-test-host", Uuid::new_v4())
            .expect("test supervisor owner"),
        policy: GatewayServiceSupervisorPolicy::default(),
        ownership: Arc::new(PostgresGatewayServiceOwnership::new(pool.clone())),
        failure_store: Arc::new(PostgresGatewayServiceFailureStore::new(pool.clone())),
        resolver: Arc::new(NoopLaunchResolver),
        provider: Arc::new(ServiceTransportProvider {
            inner: FakeProvider::new(),
            provisioned: Arc::new(AtomicUsize::new(0)),
            destroyed: Arc::clone(&destroyed),
            destroy_gate: None,
            event_sender: Arc::new(Mutex::new(None)),
            inject_events: false,
        }),
        targets: Arc::new(PostgresGatewayServiceTargets::new(pool)),
        registry: GatewayServiceRegistry::new(10, 16).expect("test service registry"),
        service_authority: String::from("127.0.0.1:8080"),
    };
    (context, destroyed)
}
