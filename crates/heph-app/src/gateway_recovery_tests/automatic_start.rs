use super::*;

// This fixture assembles the same durable ports as production so the loop can
// be tested without a public startup seam.
// This fixture wires each production port explicitly so tests cannot hide
// lifecycle dependencies behind a public test-only seam.
#[allow(clippy::too_many_arguments)]
pub(super) async fn spawn_automatic_start_with_worker(
    pool: &sqlx::PgPool,
    worker: &sqlx::PgPool,
    fixture: Fixture,
    restore_active: bool,
    retain_inventory: bool,
    desired_revision: Option<Uuid>,
    fail_revision: Option<Uuid>,
    ownership_override: Option<Arc<dyn GatewayServiceOwnership>>,
    target_override: Option<Arc<dyn GatewayServiceTargetStore>>,
    destroy_gate: Option<Arc<DestroyGate>>,
    resolver_override: Option<Arc<dyn GatewayServiceLaunchResolver>>,
    claim_resolution_override: Option<Arc<dyn GatewayServiceClaimResolutionStore>>,
) -> (
    tokio::task::JoinHandle<()>,
    CancellationToken,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned, _) =
        spawn_automatic_start_with_worker_config(
            pool,
            worker,
            fixture,
            restore_active,
            retain_inventory,
            desired_revision,
            fail_revision,
            ownership_override,
            target_override,
            destroy_gate,
            resolver_override,
            claim_resolution_override,
            None,
        )
        .await;
    (
        task,
        cancellation,
        caddy_started,
        caddy_release,
        destroyed,
        provisioned,
    )
}

// This helper deliberately moves the supervisor context into the parent loop
// future so its cleanup handles remain live until that loop settles.
#[allow(
    clippy::significant_drop_tightening,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(super) async fn spawn_automatic_start_with_worker_config(
    pool: &sqlx::PgPool,
    worker: &sqlx::PgPool,
    fixture: Fixture,
    restore_active: bool,
    retain_inventory: bool,
    desired_revision: Option<Uuid>,
    fail_revision: Option<Uuid>,
    ownership_override: Option<Arc<dyn GatewayServiceOwnership>>,
    target_override: Option<Arc<dyn GatewayServiceTargetStore>>,
    destroy_gate: Option<Arc<DestroyGate>>,
    resolver_override: Option<Arc<dyn GatewayServiceLaunchResolver>>,
    claim_resolution_override: Option<Arc<dyn GatewayServiceClaimResolutionStore>>,
    log_writer: Option<GatewayServiceLogWriterConfig>,
) -> (
    tokio::task::JoinHandle<()>,
    CancellationToken,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<Mutex<Option<tokio::sync::broadcast::Sender<VmEvent>>>>,
) {
    let host_id = format!("recovery-test-{}", fixture.gateway.simple());
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = $2,
                desired_service_revision_id = $3
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(if restore_active {
        Some(fixture.revision)
    } else {
        None
    })
    .bind(desired_revision.or_else(|| (!restore_active).then_some(fixture.revision)))
    .execute(pool)
    .await
    .expect("set automatic startup target");
    if retain_inventory {
        let instance = fixture
            .service_instance
            .expect("inventory fixture instance");
        sqlx::query(
            "DELETE FROM gateway_service_instances
              WHERE gateway_id = $1 AND revision_id = $2",
        )
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .execute(pool)
        .await
        .expect("replace inventory fixture");
        sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, $6, $4, 1, $5, 'ready',
                     now() + interval '10 minutes', now())",
        )
        .bind(instance)
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .bind(fixture.owner)
        .bind(format!("gateway-service-{instance}"))
        .bind(&host_id)
        .execute(pool)
        .await
        .expect("insert unexpired host inventory");
    } else {
        sqlx::query(
            "DELETE FROM gateway_service_instances
              WHERE gateway_id = $1 AND revision_id = $2",
        )
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .execute(pool)
        .await
        .expect("remove instance before automatic startup");
    }

    let recovery_pool = worker.clone();
    let destroyed = Arc::new(AtomicUsize::new(0));
    let provisioned = Arc::new(AtomicUsize::new(0));
    let postgres_ownership = Arc::new(PostgresGatewayServiceOwnership::new(recovery_pool.clone()));
    // The observer wraps ordinary ownership calls only; exact expired-claim
    // takeover must continue using the real Postgres recovery adapter.
    let ownership: Arc<dyn GatewayServiceOwnership> =
        ownership_override.unwrap_or_else(|| postgres_ownership.clone());
    let failure_store = Arc::new(PostgresGatewayServiceFailureStore::new(
        recovery_pool.clone(),
    ));
    let resolver: Arc<dyn GatewayServiceLaunchResolver> =
        resolver_override.unwrap_or_else(|| match fail_revision {
            Some(revision_id) => Arc::new(FailLaunchResolver { revision_id }),
            None => Arc::new(NoopLaunchResolver),
        });
    let postgres_targets = Arc::new(PostgresGatewayServiceTargets::new(recovery_pool.clone()));
    let targets: Arc<dyn GatewayServiceTargetStore> =
        target_override.unwrap_or_else(|| postgres_targets.clone());
    let limited_registry = claim_resolution_override.is_some();
    let claim_resolution: Arc<dyn GatewayServiceClaimResolutionStore> =
        claim_resolution_override.unwrap_or_else(|| postgres_ownership.clone());
    let short_cleanup_lease = destroy_gate
        .as_ref()
        .is_some_and(|gate| gate.hold_after_first_failure.load(Ordering::Acquire));
    let provider = Arc::new(ServiceTransportProvider {
        inner: FakeProvider::new(),
        provisioned: Arc::clone(&provisioned),
        destroyed: Arc::clone(&destroyed),
        destroy_gate,
        event_sender: Arc::new(Mutex::new(None)),
        inject_events: log_writer.is_some(),
    });
    let mut policy = GatewayServiceSupervisorPolicy::default();
    if short_cleanup_lease {
        // Keep the normal heartbeat path, but make expiry observable while the
        // first physical failure is held behind the test gate.
        policy.lease.lease_duration = StdDuration::from_secs(1);
        policy.lease.renewal_interval = StdDuration::from_millis(100);
    }
    if limited_registry {
        policy.serving_gateway_capacity = 2;
        policy.replacement_capacity = 1;
    }
    let owner =
        GatewayServiceOwner::new(host_id.clone(), Uuid::new_v4()).expect("automatic startup owner");
    let registry_capacity = if limited_registry { 3 } else { 10 };
    let supervisor_context = GatewayServiceSupervisorContext {
        owner: owner.clone(),
        policy,
        ownership: ownership.clone(),
        failure_store: failure_store.clone(),
        resolver: resolver.clone(),
        provider: provider.clone(),
        targets: targets.clone(),
        registry: GatewayServiceRegistry::new(registry_capacity, 16)
            .expect("automatic startup registry"),
        service_authority: String::from("127.0.0.1:8080"),
    };
    let boot = GatewayServiceBootRecovery::new(GatewayServiceBootRecoveryContext {
        owner,
        cleanup_policy: GatewayServiceCleanupDriverPolicy {
            lease: policy.lease,
            database_timeout: policy.instance.probe_timeout,
        },
        shutdown_timeout: policy.instance.shutdown_timeout,
        ownership: ownership.clone(),
        exact_recovery: postgres_ownership.clone(),
        targets: targets.clone(),
        failure_store,
        resolver,
        provider: provider.clone(),
    })
    .expect("automatic startup boot gate");
    let caddy_started = Arc::new(tokio::sync::Notify::new());
    let caddy_release = Arc::new(tokio::sync::Notify::new());
    let caddy = Arc::new(BlockingCaddyProvider {
        started: Arc::clone(&caddy_started),
        release: Arc::clone(&caddy_release),
        reconciles: Arc::new(AtomicUsize::new(0)),
    });
    let cancellation = CancellationToken::new();
    let event_sender = Arc::clone(&provider.event_sender);
    let task = tokio::spawn(gateway_reconciliation_loop_with_context(
        make_authority(pool.clone()),
        make_authority(recovery_pool),
        supervisor_context,
        boot,
        Some(claim_resolution),
        Some(
            postgres_ownership.clone() as Arc<dyn gateway_edge::GatewayServiceExpiredClaimRecovery>
        ),
        log_writer,
        targets,
        caddy,
        cancellation.clone(),
    ));
    (
        task,
        cancellation,
        caddy_started,
        caddy_release,
        destroyed,
        provisioned,
        event_sender,
    )
}
