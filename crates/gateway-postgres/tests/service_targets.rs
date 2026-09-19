//! Real `PostgreSQL` coverage for read-only persistent-service target queries.

use gateway_edge::{
    GatewayServiceIdentity, GatewayServiceInstanceKey, GatewayServiceInstancePage,
    GatewayServiceInstanceState, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceTargetPage, GatewayServiceTargetStore, MAX_SERVICE_INSTANCE_PAGE_SIZE,
    MAX_SERVICE_TARGET_PAGE_SIZE,
};
use gateway_postgres::{PostgresGatewayServiceOwnership, PostgresGatewayServiceTargets};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{collections::HashSet, env, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_targets_preserve_serving_candidate_and_lifecycle_boundaries() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let store = PostgresGatewayServiceTargets::new(worker);
    let serving_and_revoked = seed_gateway(&pool, "revoked-candidate", "enabled", "revoked").await;
    set_pointers(
        &pool,
        serving_and_revoked.gateway,
        serving_and_revoked.old_service,
        Some(serving_and_revoked.candidate_service),
    )
    .await;
    insert_accepted_invocation(&pool, &serving_and_revoked).await;

    let mixed = seed_gateway(&pool, "mixed-stateless-service", "enabled", "published").await;
    set_pointers(
        &pool,
        mixed.gateway,
        mixed.stateless,
        Some(mixed.candidate_service),
    )
    .await;
    insert_accepted_invocation(&pool, &mixed).await;

    let paused = seed_gateway(&pool, "paused-service", "paused", "published").await;
    set_pointers(
        &pool,
        paused.gateway,
        paused.old_service,
        Some(paused.candidate_service),
    )
    .await;

    let stateless_only = seed_gateway(&pool, "stateless-only", "enabled", "published").await;
    set_pointers(
        &pool,
        stateless_only.gateway,
        stateless_only.stateless,
        None,
    )
    .await;

    let listed = tokio::time::timeout(Duration::from_secs(30), async {
        let mut cursor = None;
        let mut listed = Vec::new();
        loop {
            let page = GatewayServiceTargetPage::new(cursor, 1).expect("bounded page");
            assert!(page.limit <= MAX_SERVICE_TARGET_PAGE_SIZE);
            let result = store
                .list_service_targets(page)
                .await
                .expect("list service targets");
            assert!(result.targets.len() <= 1);
            listed.extend(result.targets);
            let Some(next) = result.next_after else {
                break;
            };
            assert_ne!(Some(next), cursor);
            if let Some(previous) = cursor {
                assert!(next > previous);
            }
            cursor = Some(next);
        }
        listed
    })
    .await
    .expect("service target pagination completes");
    let listed_ids: HashSet<_> = listed.iter().map(|target| target.gateway_id).collect();
    assert!(listed_ids.contains(&serving_and_revoked.gateway));
    assert!(listed_ids.contains(&mixed.gateway));
    assert!(!listed_ids.contains(&paused.gateway));
    assert!(!listed_ids.contains(&stateless_only.gateway));

    let serving = listed
        .iter()
        .find(|target| target.gateway_id == serving_and_revoked.gateway)
        .expect("serving and candidate target");
    let active = serving
        .active_service_revision
        .as_ref()
        .expect("published serving service");
    assert_eq!(active.revision_id, serving_and_revoked.old_service);
    assert_eq!(
        active.service.log_capture_mode,
        gateway_domain::ServiceLogCaptureMode::Application
    );
    assert_eq!(active.release_state.as_deref(), Some("published"));
    assert!(active.publication_eligible);
    let candidate = serving
        .desired_service_revision
        .as_ref()
        .expect("revoked desired service");
    assert_eq!(candidate.revision_id, serving_and_revoked.candidate_service);
    assert_eq!(
        candidate.service.log_capture_mode,
        gateway_domain::ServiceLogCaptureMode::Application
    );
    assert_eq!(candidate.release_state.as_deref(), Some("revoked"));
    assert!(!candidate.publication_eligible);

    let mixed_target = listed
        .iter()
        .find(|target| target.gateway_id == mixed.gateway)
        .expect("mixed target");
    assert!(mixed_target.active_service_revision.is_none());
    assert_eq!(
        mixed_target
            .desired_service_revision
            .as_ref()
            .expect("mixed desired service")
            .revision_id,
        mixed.candidate_service
    );
    assert_eq!(
        mixed_target
            .desired_service_revision
            .as_ref()
            .expect("mixed desired service")
            .service
            .log_capture_mode,
        gateway_domain::ServiceLogCaptureMode::Application
    );

    let paused_target = store
        .get_service_target(paused.gateway, paused.old_service)
        .await
        .expect("exact paused target query")
        .expect("paused target exists");
    assert_eq!(paused_target.lifecycle, "paused");
    assert_eq!(paused_target.revision.revision_id, paused.old_service);
    assert_eq!(
        paused_target.revision.service.log_capture_mode,
        gateway_domain::ServiceLogCaptureMode::Application
    );
    assert_eq!(
        paused_target.desired_service_revision_id,
        Some(paused.candidate_service)
    );
    assert!(
        store
            .get_service_target(serving_and_revoked.gateway, mixed.candidate_service)
            .await
            .expect("cross-revision target query")
            .is_none()
    );

    assert_eq!(
        store
            .count_accepted_service_invocations(
                serving_and_revoked.gateway,
                serving_and_revoked.old_service,
            )
            .await
            .expect("accepted invocation count"),
        1
    );
    assert_eq!(
        store
            .count_accepted_service_invocations(
                serving_and_revoked.gateway,
                serving_and_revoked.candidate_service,
            )
            .await
            .expect("candidate invocation count"),
        0
    );
    assert_eq!(
        store
            .count_accepted_service_invocations(serving_and_revoked.gateway, mixed.old_service,)
            .await
            .expect("project/revision mismatch count"),
        0
    );
    let old_key = GatewayServiceInstanceKey {
        identity: GatewayServiceIdentity {
            instance_id: serving_and_revoked.old_instance,
            gateway_id: serving_and_revoked.gateway,
            revision_id: serving_and_revoked.old_service,
        },
        fencing_token: 1,
    };
    assert_eq!(
        store
            .count_accepted_service_invocations_for_instance(old_key)
            .await
            .expect("exact accepted invocation count"),
        1
    );
    assert_eq!(
        store
            .count_accepted_service_invocations_for_instance(GatewayServiceInstanceKey {
                fencing_token: 2,
                ..old_key
            })
            .await
            .expect("stale fence accepted invocation count"),
        0
    );
    let mixed_key = GatewayServiceInstanceKey {
        identity: GatewayServiceIdentity {
            instance_id: mixed.old_instance,
            gateway_id: mixed.gateway,
            revision_id: mixed.old_service,
        },
        fencing_token: 1,
    };
    assert_eq!(
        store
            .count_accepted_service_invocations_for_instance(mixed_key)
            .await
            .expect("different instance accepted invocation count"),
        1
    );
    assert_eq!(
        store
            .count_accepted_service_invocations_for_instance(old_key)
            .await
            .expect("original instance count remains isolated"),
        1
    );
    assert!(
        store
            .count_accepted_service_invocations_for_instance(GatewayServiceInstanceKey {
                identity: GatewayServiceIdentity {
                    instance_id: Uuid::nil(),
                    ..old_key.identity
                },
                fencing_token: 1,
            })
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_service_instance_lookup_survives_fencing_and_cleanup() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let store = PostgresGatewayServiceTargets::new(worker.clone());
    let fixture = seed_gateway_with_instance_state(
        &pool,
        "exact-instance",
        "enabled",
        "published",
        "starting",
        true,
        None,
    )
    .await;
    let identity = GatewayServiceIdentity {
        instance_id: fixture.old_instance,
        gateway_id: fixture.gateway,
        revision_id: fixture.old_service,
    };

    let observed = store
        .get_service_instance(identity)
        .await
        .expect("lookup starting instance")
        .expect("starting instance exists");
    assert_eq!(observed.identity, identity);
    assert_eq!(observed.state, GatewayServiceInstanceState::Starting);
    assert_eq!(observed.fencing_token, 1);

    sqlx::query("UPDATE gateway_service_instances SET state = 'stopping' WHERE id = $1")
        .bind(fixture.old_instance)
        .execute(&pool)
        .await
        .expect("transition instance to stopping");
    let observed = store
        .get_service_instance(identity)
        .await
        .expect("lookup stopping instance")
        .expect("stopping instance exists");
    assert_eq!(observed.state, GatewayServiceInstanceState::Stopping);

    let recovery_owner =
        GatewayServiceOwner::new(&fixture.owner_host_id, Uuid::new_v4()).expect("owner");
    let ownership = PostgresGatewayServiceOwnership::new(worker);
    let recovered = ownership
        .claim_expired(&recovery_owner, Duration::from_secs(30), 1)
        .await
        .expect("claim expired instance");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].identity, identity);
    assert_eq!(recovered[0].fencing_token, 2);
    assert_eq!(recovered[0].owner_uuid, recovery_owner.owner_uuid);
    assert_eq!(recovered[0].state, GatewayServiceInstanceState::Stopping);

    let observed = store
        .get_service_instance(identity)
        .await
        .expect("lookup recovered instance")
        .expect("recovered instance exists");
    assert_eq!(observed.fencing_token, 2);
    assert_eq!(observed.owner_uuid, recovery_owner.owner_uuid);
    assert_eq!(observed.state, GatewayServiceInstanceState::Stopping);

    ownership
        .mark_cleaned(&recovered[0], &recovery_owner)
        .await
        .expect("mark instance cleaned");
    let cleaned = store
        .get_service_instance(identity)
        .await
        .expect("lookup cleaned instance")
        .expect("cleaned instance remains queryable");
    assert_eq!(cleaned.state, GatewayServiceInstanceState::Cleaned);
    assert_eq!(cleaned.identity, identity);

    assert!(
        store
            .get_service_instance(GatewayServiceIdentity {
                gateway_id: Uuid::new_v4(),
                ..identity
            })
            .await
            .expect("wrong gateway lookup")
            .is_none()
    );
    assert!(
        store
            .get_service_instance(GatewayServiceIdentity {
                revision_id: Uuid::new_v4(),
                ..identity
            })
            .await
            .expect("wrong revision lookup")
            .is_none()
    );
    assert!(
        store
            .get_service_instance(GatewayServiceIdentity {
                instance_id: Uuid::new_v4(),
                ..identity
            })
            .await
            .expect("unknown instance lookup")
            .is_none()
    );
    for invalid_identity in [
        GatewayServiceIdentity {
            instance_id: Uuid::nil(),
            ..identity
        },
        GatewayServiceIdentity {
            gateway_id: Uuid::nil(),
            ..identity
        },
        GatewayServiceIdentity {
            revision_id: Uuid::nil(),
            ..identity
        },
    ] {
        assert!(store.get_service_instance(invalid_identity).await.is_err());
    }
}

#[test]
fn service_target_page_rejects_unbounded_or_nil_cursors() {
    assert!(GatewayServiceTargetPage::new(None, 1).is_ok());
    assert!(GatewayServiceTargetPage::new(None, MAX_SERVICE_TARGET_PAGE_SIZE).is_ok());
    assert!(GatewayServiceTargetPage::new(None, 0).is_err());
    assert!(GatewayServiceTargetPage::new(None, MAX_SERVICE_TARGET_PAGE_SIZE + 1).is_err());
    assert!(GatewayServiceTargetPage::new(Some(Uuid::nil()), 1).is_err());
    assert!(GatewayServiceInstancePage::new("valid-host", None, 1).is_ok());
    assert!(GatewayServiceInstancePage::new("", None, 1).is_err());
    assert!(GatewayServiceInstancePage::new("invalid host", None, 1).is_err());
    assert!(GatewayServiceInstancePage::new("valid-host", None, 0).is_err());
    assert!(GatewayServiceInstancePage::new("valid-host", Some(Uuid::nil()), 1).is_err());
    assert!(
        GatewayServiceInstancePage::new("valid-host", None, MAX_SERVICE_INSTANCE_PAGE_SIZE + 1)
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_instance_inventory_is_stable_by_host_and_cursor() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let store = PostgresGatewayServiceTargets::new(worker);
    let host = format!("inventory-host-{}", Uuid::new_v4());
    let mut expected = HashSet::new();
    let mut expired_instance = None;
    for index in 0..130 {
        let fixture = seed_gateway_with_instance_state(
            &pool,
            &format!("inventory-{index}-{}", Uuid::new_v4()),
            "enabled",
            "published",
            "ready",
            index == 0,
            Some(&host),
        )
        .await;
        if index == 0 {
            expired_instance = Some(fixture.old_instance);
        }
        expected.insert(fixture.old_instance);
    }
    let foreign_host = format!("foreign-host-{}", Uuid::new_v4());
    let foreign = seed_gateway_with_instance_state(
        &pool,
        &format!("foreign-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "ready",
        false,
        Some(&foreign_host),
    )
    .await;
    let cleaned = seed_gateway_with_instance_state(
        &pool,
        &format!("cleaned-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "cleaned",
        false,
        Some(&host),
    )
    .await;

    let observed = tokio::time::timeout(Duration::from_secs(30), async {
        let mut cursor = None;
        let mut pages = 0;
        let mut observed = Vec::new();
        loop {
            pages += 1;
            assert!(pages <= 16, "inventory pagination exceeded bounded pages");
            let page = GatewayServiceInstancePage::new(host.clone(), cursor, 17)
                .expect("valid inventory page");
            let result = store
                .list_service_instances(page)
                .await
                .expect("list service instances");
            assert!(result.instances.len() <= 17);
            observed.extend(result.instances);
            let Some(next) = result.next_after else {
                break;
            };
            assert_ne!(Some(next), cursor);
            cursor = Some(next);
        }
        observed
    })
    .await
    .expect("inventory pagination completes");
    assert!(
        observed
            .windows(2)
            .all(|pair| pair[0].identity.instance_id < pair[1].identity.instance_id)
    );
    let observed_ids: HashSet<_> = observed
        .iter()
        .map(|lease| lease.identity.instance_id)
        .collect();
    assert_eq!(observed.len(), expected.len());
    assert_eq!(observed_ids, expected);
    let expired_instance = expired_instance.expect("expired inventory row");
    assert!(observed.iter().any(|lease| {
        lease.identity.instance_id == expired_instance
            && lease.lease_expires_at < OffsetDateTime::now_utc()
    }));
    assert!(
        observed
            .iter()
            .any(|lease| lease.lease_expires_at > OffsetDateTime::now_utc())
    );
    let owners: HashSet<_> = observed.iter().map(|lease| lease.owner_uuid).collect();
    assert!(owners.len() > 1);
    assert!(!observed_ids.contains(&foreign.old_instance));
    assert!(!observed_ids.contains(&cleaned.old_instance));
}

struct Fixture {
    project: Uuid,
    gateway: Uuid,
    old_service: Uuid,
    candidate_service: Uuid,
    stateless: Uuid,
    old_route: Uuid,
    old_instance: Uuid,
    owner_host_id: String,
}

async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway migrations");
    let max_version: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&pool)
    .await
    .expect("read latest migration")
    .expect("migrations are present");
    assert!(max_version >= 74);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_version}");
    Some(pool)
}

async fn worker_pool() -> sqlx::PgPool {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-targets-test'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker PostgreSQL pool");
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("read worker current user");
    assert_eq!(current_user, "hephaestus_worker");
    pool
}

// This fixture keeps the complete gateway/release graph in one setup helper so
// each boundary test uses the same valid persisted shape.
#[allow(clippy::too_many_lines)]
async fn seed_gateway(
    pool: &sqlx::PgPool,
    name: &str,
    lifecycle: &str,
    candidate_state: &str,
) -> Fixture {
    seed_gateway_with_instance_state(pool, name, lifecycle, candidate_state, "ready", false, None)
        .await
}

// This variant lets lifecycle-boundary tests start an instance in the exact
// persisted state needed to exercise the database transition trigger.
#[allow(clippy::too_many_lines)]
async fn seed_gateway_with_instance_state(
    pool: &sqlx::PgPool,
    name: &str,
    lifecycle: &str,
    candidate_state: &str,
    instance_state: &str,
    instance_expired: bool,
    owner_host_override: Option<&str>,
) -> Fixture {
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let old_service = Uuid::new_v4();
    let candidate_service = Uuid::new_v4();
    let stateless = Uuid::new_v4();
    let old_route = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(owner)
        .bind(format!("target-owner-{name}"))
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("target-org-{name}-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("target-project-{name}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("target-repository-{name}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query(
        "INSERT INTO gateways
            (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(format!("target-{name}"))
    .bind(lifecycle)
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway");

    let old_release = insert_release(pool, repository, owner, name, "published").await;
    let candidate_release = insert_release(
        pool,
        repository,
        owner,
        &format!("{name}-candidate"),
        candidate_state,
    )
    .await;
    insert_revision(
        pool,
        old_service,
        gateway,
        project,
        repository,
        owner,
        Some(old_release),
        "http.service.v1",
        [1; 32],
    )
    .await;
    insert_revision(
        pool,
        candidate_service,
        gateway,
        project,
        repository,
        owner,
        Some(candidate_release),
        "http.service.v1",
        [2; 32],
    )
    .await;
    insert_revision(
        pool, stateless, gateway, project, repository, owner, None, "http.v1", [3; 32],
    )
    .await;
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/target', ARRAY['GET'])",
    )
    .bind(old_route)
    .bind(old_service)
    .bind(gateway)
    .bind(project)
    .execute(pool)
    .await
    .expect("service route");
    let old_instance = Uuid::new_v4();
    let owner_host_id =
        owner_host_override.map_or_else(|| format!("target-host-{name}-{gateway}"), str::to_owned);
    let lease_expires = if instance_expired {
        "now() - interval '1 second'"
    } else {
        "now() + interval '10 minutes'"
    };
    let heartbeat_at = if instance_expired {
        "now() - interval '2 seconds'"
    } else {
        "now()"
    };
    let query = format!(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid,
             fencing_token, vm_id, state, lease_expires_at, heartbeat_at, cleaned_at)
         VALUES ($1, $2, $3, $7, $4, 1,
                 $5, $6, {lease_expires}, {heartbeat_at},
                 CASE WHEN $6 = 'cleaned' THEN now() ELSE NULL END)"
    );
    sqlx::query(&query)
        .bind(old_instance)
        .bind(gateway)
        .bind(old_service)
        .bind(owner)
        .bind(format!("gateway-service-{old_instance}"))
        .bind(instance_state)
        .bind(&owner_host_id)
        .execute(pool)
        .await
        .expect("ready service instance");
    Fixture {
        project,
        gateway,
        old_service,
        candidate_service,
        stateless,
        old_route,
        old_instance,
        owner_host_id,
    }
}

async fn insert_release(
    pool: &sqlx::PgPool,
    repository: Uuid,
    owner: Uuid,
    version: &str,
    state: &str,
) -> Uuid {
    let release = Uuid::new_v4();
    let build = Uuid::new_v4();
    let source_commit = format!("00000000{}", release.simple());
    sqlx::query(
        "INSERT INTO build_requests
            (id, repository_id, source_commit, source_ref, build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build)
    .bind(repository)
    .bind(&source_commit)
    .bind([4_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("build request");
    let publication = if state == "published" {
        "now()"
    } else {
        "NULL"
    };
    let revoked = if state == "revoked" { "now()" } else { "NULL" };
    let query = format!(
        "INSERT INTO releases
            (id, repository_id, version, source_commit, source_ref, build_request_id,
             build_definition_hash, configuration, configuration_hash, manifest_hash,
             state, publication_actor_id, published_at, revoked_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{{}}', $7, $8, $9, $10,
                 {publication}, {revoked})"
    );
    sqlx::query(&query)
        .bind(release)
        .bind(repository)
        .bind(version)
        .bind(&source_commit)
        .bind(build)
        .bind([4_u8; 32].as_slice())
        .bind([5_u8; 32].as_slice())
        .bind([6_u8; 32].as_slice())
        .bind(state)
        .bind(owner)
        .execute(pool)
        .await
        .expect("release");
    let family = Uuid::new_v4();
    let agent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(family)
    .bind(repository)
    .bind(format!("target-agent-{release}"))
    .execute(pool)
    .await
    .expect("release agent family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name, runtime_contract,
             runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'target-service', 'Target service', '{}', $4, '[]', '[]', false)",
    )
    .bind(agent)
    .bind(release)
    .bind(family)
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("release agent");
    release
}

// The SQL fixture mirrors the revision identity columns directly for clarity.
#[allow(clippy::too_many_arguments)]
async fn insert_revision(
    pool: &sqlx::PgPool,
    revision: Uuid,
    gateway: Uuid,
    project: Uuid,
    repository: Uuid,
    owner: Uuid,
    release: Option<Uuid>,
    contract: &str,
    hash: [u8; 32],
) {
    let (port, readiness, health) = if contract == "http.service.v1" {
        (Some(18080_i32), Some("/ready"), Some("/health"))
    } else {
        (None, None, None)
    };
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
             release_agent_key, handler_contract, exposure, parameters,
             service_loopback_port, service_readiness_path, service_health_path,
             service_log_capture_mode, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5,
                 CASE WHEN $5 IS NULL THEN NULL ELSE
                     (SELECT id FROM release_agents WHERE release_id = $5 LIMIT 1)
                 END,
                 CASE WHEN $5 IS NULL THEN NULL ELSE 'target-service' END,
                 $6, 'public', '{}', $7, $8, $9,
                 CASE WHEN $6 = 'http.service.v1' THEN 'application' ELSE 'disabled' END,
                 $10, $11)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(release)
    .bind(contract)
    .bind(port)
    .bind(readiness)
    .bind(health)
    .bind(hash.as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway revision");
}

async fn set_pointers(pool: &sqlx::PgPool, gateway: Uuid, active: Uuid, desired: Option<Uuid>) {
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = $2, desired_service_revision_id = $3
          WHERE id = $1",
    )
    .bind(gateway)
    .bind(active)
    .bind(desired)
    .execute(pool)
    .await
    .expect("gateway pointers");
}

async fn insert_accepted_invocation(pool: &sqlx::PgPool, fixture: &Fixture) {
    sqlx::query(
        "INSERT INTO gateway_invocations
            (id, gateway_id, gateway_revision_id, gateway_route_id, project_id,
             request_id, outcome, service_instance_id,
             service_instance_fencing_token)
         VALUES ($1, $2, $3, $4, $5, $6, 'accepted', $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.gateway)
    .bind(fixture.old_service)
    .bind(fixture.old_route)
    .bind(fixture.project)
    .bind(Uuid::new_v4())
    .bind(fixture.old_instance)
    .bind(1_i64)
    .execute(pool)
    .await
    .expect("accepted invocation");
}
