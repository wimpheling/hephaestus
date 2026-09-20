//! Real `PostgreSQL` UI gateway authority and worker privilege matrix.
//!
//! These tests use the same published release and installation fixture shape
//! as the browser authentication schema tests, then exercise the gateway
//! worker adapter through its public edge ports.

use gateway_edge::{
    GatewayInvocationRecorder, GatewayLimits, GatewayScheme, TrustedRequestMetadata,
    UiGatewayAdmissionProvider, UiGatewayAuthority, UiGatewayRequest, UiGatewayRequestKind,
};
use gateway_postgres::PostgresGatewayEdgeAuthority;
use http::{HeaderMap, Method};
use serial_test::serial;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{env, net::IpAddr, str::FromStr, time::Duration};
use uuid::Uuid;

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_gateway_worker_admits_managed_and_api_and_fails_closed() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI gateway authority: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply UI gateway migrations");
    let worker = worker_pool(&database_url).await;
    let app = app_pool(&database_url).await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let gateway = PostgresGatewayEdgeAuthority::new(worker.clone(), limits());

    let managed_child = insert_managed_authenticated_child(&worker, &fixture, [94; 32]).await;
    let managed_authority = authority(
        managed_child,
        fixture.managed_installation,
        fixture.managed_generation,
        fixture.actor,
        fixture.organization,
        "docs/subpage",
        UiGatewayRequestKind::Managed,
        Method::GET,
    );
    let managed_admission = gateway
        .admit(&request(managed_authority.clone()))
        .await
        .expect("managed descendant is admitted");
    assert_eq!(
        managed_admission.route.gateway_revision_id,
        fixture.managed_revision
    );
    assert_eq!(
        managed_admission.gateway_path_and_query,
        "/gateway/service/subpage"
    );
    let sibling = authority(
        managed_child,
        fixture.managed_installation,
        fixture.managed_generation,
        fixture.actor,
        fixture.organization,
        "docs-other/asset.js",
        UiGatewayRequestKind::Managed,
        Method::GET,
    );
    assert!(gateway.admit(&request(sibling)).await.is_err());
    let managed_asset = authority(
        managed_child,
        fixture.managed_installation,
        fixture.managed_generation,
        fixture.actor,
        fixture.organization,
        "docs/assets/app.js",
        UiGatewayRequestKind::Managed,
        Method::GET,
    );
    let mut managed_asset_request = request(managed_asset);
    managed_asset_request.request_path_and_query.push('?');
    let managed_asset_admission = gateway
        .admit(&managed_asset_request)
        .await
        .expect("managed asset descendant is admitted");
    assert_eq!(
        managed_asset_admission.gateway_path_and_query,
        "/gateway/service/assets/app.js?"
    );

    let api_child =
        insert_authenticated_child(&worker, &fixture, [90; 32].into_iter().collect(), "1 hour")
            .await;
    let api_authority = authority(
        api_child,
        fixture.installation,
        fixture.generation,
        fixture.actor,
        fixture.organization,
        "/service/api",
        UiGatewayRequestKind::Api,
        Method::POST,
    );
    let api_admission = gateway
        .admit(&request(api_authority.clone()))
        .await
        .expect("exact API route is admitted");
    assert_eq!(api_admission.gateway_path_and_query, "/gateway/service/api");

    let relative_api = authority(
        api_child,
        fixture.installation,
        fixture.generation,
        fixture.actor,
        fixture.organization,
        "service/api",
        UiGatewayRequestKind::Api,
        Method::POST,
    );
    assert!(gateway.admit(&request(relative_api)).await.is_err());

    let mut wrong_actor = api_authority.clone();
    wrong_actor.actor_id = fixture.outsider;
    assert!(gateway.admit(&request(wrong_actor)).await.is_err());
    let mut wrong_method = api_authority.clone();
    wrong_method.method = Method::GET;
    assert!(gateway.admit(&request(wrong_method)).await.is_err());

    let before_identity_recheck: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_invocations")
            .fetch_one(&worker)
            .await
            .expect("count invocations before accepted UI identity recheck");
    let mut wrong_managed_actor = managed_authority.clone();
    wrong_managed_actor.actor_id = fixture.outsider;
    assert!(
        gateway
            .accepted_ui(
                &managed_admission.route,
                &wrong_managed_actor,
                Uuid::new_v4(),
            )
            .await
            .is_err()
    );
    let after_identity_recheck: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_invocations")
            .fetch_one(&worker)
            .await
            .expect("count invocations after accepted UI identity recheck");
    assert_eq!(
        after_identity_recheck, before_identity_recheck,
        "accepted UI identity recheck must precede insertion"
    );

    let replacement_generation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 2, $3, 'schema-ui', 'project')",
    )
    .bind(replacement_generation)
    .bind(fixture.installation)
    .bind(fixture.release)
    .execute(&worker)
    .await
    .expect("insert stale-generation replacement");
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(replacement_generation)
        .execute(&worker)
        .await
        .expect("move installation generation");
    assert!(
        gateway
            .admit(&request(api_authority.clone()))
            .await
            .is_err()
    );
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(fixture.generation)
        .execute(&worker)
        .await
        .expect("restore installation generation");

    let unbound_child = insert_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.other_installation,
        fixture.other_generation,
        "schema-ui-two",
        [91_u8; 32].into_iter().collect(),
    )
    .await;
    let unbound_authority = authority(
        unbound_child,
        fixture.other_installation,
        fixture.other_generation,
        fixture.actor,
        fixture.organization,
        "/service/api",
        UiGatewayRequestKind::Api,
        Method::POST,
    );
    assert!(gateway.admit(&request(unbound_authority)).await.is_err());

    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.managed_gateway)
        .execute(&worker)
        .await
        .expect("pause gateway");
    assert!(
        gateway
            .admit(&request(managed_authority.clone()))
            .await
            .is_err()
    );
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.managed_gateway)
        .execute(&worker)
        .await
        .expect("restore gateway");

    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM gateway_invocations")
        .fetch_one(&worker)
        .await
        .expect("count invocations before service denial");
    assert!(
        gateway
            .accepted_ui(&managed_admission.route, &managed_authority, Uuid::new_v4(),)
            .await
            .is_err()
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM gateway_invocations")
        .fetch_one(&worker)
        .await
        .expect("count invocations after service denial");
    assert_eq!(after, before, "service admission fails before insertion");

    let worker_verifier: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authenticate_ui_browser_session($1, $2, 'api', '/service/api', 'POST')",
    )
    .bind(vec![90_u8; 32])
    .bind(fixture.generation)
    .fetch_one(&worker)
    .await
    .expect("worker has the explicit verifier grant");
    assert_eq!(worker_verifier, 1);
    let app_verifier: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authenticate_ui_browser_session($1, $2, 'api', '/service/api', 'POST')",
    )
    .bind(vec![90_u8; 32])
    .bind(fixture.generation)
    .fetch_one(&app)
    .await
    .expect("application retains the original verifier grant");
    assert_eq!(app_verifier, 1);
    assert!(
        sqlx::query("SELECT id FROM ui_browser_sessions LIMIT 1")
            .fetch_one(&app)
            .await
            .is_err()
    );
    sqlx::query("UPDATE human_browser_sessions SET revoked_at = statement_timestamp(), revocation_reason = 'logout' WHERE id = $1")
        .bind(fixture.parent_session)
        .execute(&worker)
        .await
        .expect("revoke parent");
    assert!(gateway.admit(&request(api_authority)).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn ui_gateway_rechecks_parent_after_service_instance_wait() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI gateway wait recheck: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply UI gateway migrations");
    let observed_worker = observed_worker_pool(&database_url).await;
    let observer = observed_activity_pool(&database_url).await;
    let fixture = seed_fixture_reusing_installation_helpers(&observed_worker).await;
    let instance_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid, fencing_token,
             vm_id, state, lease_expires_at, heartbeat_at)
         VALUES ($1, $2, $3, 'ui-gateway-wait-test', $4, 1, $5, 'ready',
                 now() + interval '10 minutes', now())",
    )
    .bind(instance_id)
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .bind(Uuid::new_v4())
    .bind(format!("gateway-service-{instance_id}"))
    .execute(&observed_worker)
    .await
    .expect("seed ready service instance");
    let gateway = PostgresGatewayEdgeAuthority::new(observed_worker.clone(), limits());
    let child = insert_managed_authenticated_child(&observed_worker, &fixture, [95; 32]).await;
    let managed_authority = authority(
        child,
        fixture.managed_installation,
        fixture.managed_generation,
        fixture.actor,
        fixture.organization,
        "docs/wait-proof",
        UiGatewayRequestKind::Managed,
        Method::GET,
    );
    let admission = gateway
        .admit(&request(managed_authority.clone()))
        .await
        .expect("managed wait-proof route is admitted");
    let mut blocker = observed_worker
        .begin()
        .await
        .expect("begin named service lock blocker");
    sqlx::query(
        "SELECT id
           FROM gateway_service_instances
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(instance_id)
    .execute(&mut *blocker)
    .await
    .expect("lock service instance for observed blocker");
    let invocation = tokio::spawn({
        let gateway = gateway.clone();
        let route = admission.route.clone();
        let managed_authority = managed_authority.clone();
        async move {
            gateway
                .accepted_ui(&route, &managed_authority, Uuid::new_v4())
                .await
        }
    });
    wait_for_service_instance_wait(&observer).await;
    sqlx::query(
        "UPDATE human_browser_sessions
            SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
          WHERE id = $1",
    )
    .bind(fixture.parent_session)
    .execute(&observed_worker)
    .await
    .expect("revoke parent while service lock is held");
    blocker
        .commit()
        .await
        .expect("release named service lock blocker");
    assert!(
        invocation
            .await
            .expect("wait-proof invocation task")
            .is_err()
    );
    let invocation_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_invocations
          WHERE gateway_id = $1 AND gateway_revision_id = $2",
    )
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .fetch_one(&observed_worker)
    .await
    .expect("count wait-proof invocations");
    assert_eq!(
        invocation_count, 0,
        "revoked parent must produce no invocation"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn ui_gateway_rechecks_child_expiry_after_service_instance_wait() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI gateway child expiry wait recheck: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply UI gateway migrations");
    let observed_worker = observed_worker_pool(&database_url).await;
    let observer = observed_activity_pool(&database_url).await;
    let fixture = seed_fixture_reusing_installation_helpers(&observed_worker).await;
    let instance_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid, fencing_token,
             vm_id, state, lease_expires_at, heartbeat_at)
         VALUES ($1, $2, $3, 'ui-gateway-child-expiry-test', $4, 1, $5, 'ready',
                 now() + interval '10 minutes', now())",
    )
    .bind(instance_id)
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .bind(Uuid::new_v4())
    .bind(format!("gateway-service-{instance_id}"))
    .execute(&observed_worker)
    .await
    .expect("seed ready child expiry service instance");
    let gateway = PostgresGatewayEdgeAuthority::new(observed_worker.clone(), limits());
    let child = insert_managed_authenticated_child_with_expiry(
        &observed_worker,
        &fixture,
        [96; 32],
        "2 seconds",
    )
    .await;
    let managed_authority = authority(
        child,
        fixture.managed_installation,
        fixture.managed_generation,
        fixture.actor,
        fixture.organization,
        "docs/child-expiry-proof",
        UiGatewayRequestKind::Managed,
        Method::GET,
    );
    let admission = gateway
        .admit(&request(managed_authority.clone()))
        .await
        .expect("managed child expiry route is admitted");
    let mut blocker = observed_worker
        .begin()
        .await
        .expect("begin child expiry service lock blocker");
    sqlx::query(
        "SELECT id
           FROM gateway_service_instances
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(instance_id)
    .execute(&mut *blocker)
    .await
    .expect("lock child expiry service instance");
    let invocation = tokio::spawn({
        let gateway = gateway.clone();
        let route = admission.route.clone();
        let managed_authority = managed_authority.clone();
        async move {
            gateway
                .accepted_ui(&route, &managed_authority, Uuid::new_v4())
                .await
        }
    });
    wait_for_service_instance_wait(&observer).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    blocker
        .commit()
        .await
        .expect("release child expiry service lock blocker");
    assert!(
        invocation
            .await
            .expect("child expiry invocation task")
            .is_err()
    );
    let invocation_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_invocations
          WHERE gateway_id = $1 AND gateway_revision_id = $2",
    )
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .fetch_one(&observed_worker)
    .await
    .expect("count child expiry invocations");
    assert_eq!(
        invocation_count, 0,
        "expired child must produce no invocation"
    );
}

async fn observed_worker_pool(database_url: &str) -> PgPool {
    let options = PgConnectOptions::from_str(database_url)
        .expect("parse observed worker database URL")
        .application_name("ui-gateway-service-instance-blocker");
    PgPoolOptions::new()
        .max_connections(6)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .expect("connect observed worker pool")
}

async fn observed_activity_pool(database_url: &str) -> PgPool {
    let options = PgConnectOptions::from_str(database_url)
        .expect("parse observer database URL")
        .application_name("ui-gateway-service-instance-observer");
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("connect privileged activity observer pool")
}

async fn wait_for_service_instance_wait(pool: &PgPool) {
    for _ in 0..100 {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1
                   FROM pg_stat_activity
                  WHERE application_name = 'ui-gateway-service-instance-blocker'
                    AND wait_event_type = 'Lock'
             )",
        )
        .fetch_one(pool)
        .await
        .expect("observe service instance blocker wait");
        if waiting {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let activity: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT application_name, wait_event_type, query
           FROM pg_stat_activity
          WHERE application_name = 'ui-gateway-service-instance-blocker'",
    )
    .fetch_all(pool)
    .await
    .expect("inspect named service instance blocker activity");
    panic!("named gateway service instance blocker was not observed: {activity:?}");
}

const fn limits() -> GatewayLimits {
    GatewayLimits {
        max_request_body_bytes: 16 * 1024,
        max_response_body_bytes: 16 * 1024,
        max_request_headers: 64,
        max_response_headers: 64,
        max_path_and_query_bytes: 4096,
        execution_timeout: Duration::from_secs(5),
    }
}

#[allow(clippy::too_many_arguments)]
fn authority(
    child_session_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    actor_id: Uuid,
    organization_id: Uuid,
    path: &str,
    request_kind: UiGatewayRequestKind,
    method: Method,
) -> UiGatewayAuthority {
    UiGatewayAuthority {
        child_session_id,
        actor_id,
        organization_id,
        installation_id,
        generation_id,
        canonical_request_path: path.to_owned(),
        request_kind,
        method,
    }
}

// The generated request body type is an indirect dependency; keep its
// default construction generic at this adapter boundary.
#[allow(clippy::default_trait_access)]
fn request(authority: UiGatewayAuthority) -> UiGatewayRequest {
    UiGatewayRequest {
        method: authority.method.clone(),
        request_path_and_query: authority.canonical_request_path.clone(),
        authority,
        headers: HeaderMap::new(),
        body: Default::default(),
        trusted: TrustedRequestMetadata {
            scheme: GatewayScheme::Https,
            authority: "ui.heph.test".to_owned(),
            client_address: "127.0.0.1".parse::<IpAddr>().expect("loopback address"),
            request_id: Uuid::new_v4(),
        },
    }
}

async fn worker_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect role pool")
}

async fn app_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect role pool")
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Fixture {
    actor: Uuid,
    outsider: Uuid,
    organization: Uuid,
    project: Uuid,
    source_project: Uuid,
    release: Uuid,
    release_agent: Uuid,
    other_organization: Uuid,
    parent_session: Uuid,
    outsider_parent_session: Uuid,
    installation: Uuid,
    other_installation: Uuid,
    generation: Uuid,
    other_generation: Uuid,
    global_installation: Uuid,
    global_generation: Uuid,
    repository_installation: Uuid,
    repository_generation: Uuid,
    managed_installation: Uuid,
    managed_generation: Uuid,
    managed_gateway: Uuid,
    managed_revision: Uuid,
    route: &'static str,
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::cognitive_complexity)]
async fn seed_fixture_reusing_installation_helpers(worker: &PgPool) -> Fixture {
    let actor = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let other_organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let source_project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let receive = Uuid::new_v4();
    let build = Uuid::new_v4();
    let source_revision = Uuid::new_v4();
    let release = Uuid::new_v4();
    let artifact = Uuid::new_v4();
    let parent_session = Uuid::new_v4();
    let outsider_parent_session = Uuid::new_v4();
    let installation = Uuid::new_v4();
    let other_installation = Uuid::new_v4();
    let generation = Uuid::new_v4();
    let other_generation = Uuid::new_v4();
    let global_installation = Uuid::new_v4();
    let global_generation = Uuid::new_v4();
    let repository_installation = Uuid::new_v4();
    let repository_generation = Uuid::new_v4();
    let managed_installation = Uuid::new_v4();
    let managed_generation = Uuid::new_v4();
    let managed_gateway = Uuid::new_v4();
    let managed_revision = Uuid::new_v4();
    let release_agent = Uuid::new_v4();
    let agent_family = Uuid::new_v4();
    let request_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO users (id, display_name) VALUES ($1, 'UI browser actor'), ($2, 'UI browser outsider')",
    )
    .bind(actor)
    .bind(outsider)
    .execute(worker)
    .await
    .expect("seed users");
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(actor.to_string())
        .execute(worker)
        .await
        .expect("set fixture actor");
    sqlx::query(
        "INSERT INTO organizations (id, name) VALUES ($1, 'UI browser org'), ($2, 'Other org')",
    )
    .bind(organization)
    .bind(other_organization)
    .execute(worker)
    .await
    .expect("seed organizations");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed organization owner");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
         VALUES ($1, $2, 'ui-browser-target'), ($3, $2, 'ui-browser-source')",
    )
    .bind(project)
    .bind(organization)
    .bind(source_project)
    .execute(worker)
    .await
    .expect("seed project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'ui-browser-repository')",
    )
    .bind(repository)
    .bind(source_project)
    .execute(worker)
    .await
    .expect("seed repository");
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'ui-browser-schema', 'accepted', now())",
    )
    .bind(receive)
    .bind(repository)
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed accepted receive");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'succeeded', $6)",
    )
    .bind(build)
    .bind(repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(receive)
    .bind(vec![1_u8; 32])
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed build request");
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config, normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', '{}'::jsonb, $7)",
    )
    .bind(source_revision)
    .bind(repository)
    .bind(receive)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    .bind(vec![2_u8; 32])
    .bind(vec![3_u8; 32])
    .execute(worker)
    .await
    .expect("seed source manifest revision");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit, source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(build)
    .bind(repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(source_revision)
    .execute(worker)
    .await
    .expect("link source manifest revision");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref, build_request_id,
          build_definition_hash, configuration, configuration_hash, manifest_hash, state)
         VALUES ($1, $2, 'ui-browser-v1', $3, 'refs/heads/main', $4, $5,
                 '{}'::jsonb, $6, $7, 'draft')",
    )
    .bind(release)
    .bind(repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(build)
    .bind(vec![1_u8; 32])
    .bind(vec![2_u8; 32])
    .bind(vec![3_u8; 32])
    .execute(worker)
    .await
    .expect("seed published release");
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(release)
    .bind(build)
    .bind(source_revision)
    .execute(worker)
    .await
    .expect("seed release UI snapshot");
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes,
          media_type, storage_key)
         VALUES ($1, $2, 'dist/index.html', 'file', 420, $3, 12,
                 'text/html', $4)",
    )
    .bind(artifact)
    .bind(release)
    .bind([7_u8; 32].as_slice())
    .bind(Uuid::new_v4())
    .execute(worker)
    .await
    .expect("seed static UI artifact");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'browser-service')",
    )
    .bind(agent_family)
    .bind(repository)
    .execute(worker)
    .await
    .expect("seed browser service family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'browser-service', 'Browser service',
                 '{}'::jsonb, $4, '[]'::jsonb, '[]'::jsonb, false)",
    )
    .bind(release_agent)
    .bind(release)
    .bind(agent_family)
    .bind(vec![4_u8; 32])
    .execute(worker)
    .await
    .expect("seed browser service release agent");
    for (ui_key, route_base, scope) in [
        ("schema-ui", "schema-ui", "project"),
        ("schema-ui-two", "schema-ui-two", "project"),
        ("schema-global", "schema-global", "global"),
        ("schema-repository", "schema-repository", "repository"),
    ] {
        sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, $2, $5, $3, 'app', 'iframe', $4,
                     'index.html', 1, 'no_store', 'static')",
        )
        .bind(release)
        .bind(ui_key)
        .bind(ui_key)
        .bind(route_base)
        .bind(scope)
        .execute(worker)
        .await
        .expect("seed release UI descriptor");
    }
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'schema-managed', 'project', 'schema-managed', 'app',
                 'iframe', 'docs', 'index.html', 1, 'no_store',
                 'managed_service')",
    )
    .bind(release)
    .execute(worker)
    .await
    .expect("seed managed UI descriptor");
    sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'schema-managed', 'browser-service', '/service', $2)",
    )
    .bind(release)
    .bind(release_agent)
    .execute(worker)
    .await
    .expect("seed managed UI binding");
    sqlx::query(
        "INSERT INTO release_ui_api_bindings
         (release_id, ui_key, api_key, gateway_name, method, route,
          release_agent_id)
         VALUES ($1, 'schema-ui', 'status', 'browser-service', 'POST',
                 '/service/api', $2),
                ($1, 'schema-managed', 'status', 'browser-service', 'POST',
                 '/service/api', $2)",
    )
    .bind(release)
    .bind(release_agent)
    .execute(worker)
    .await
    .expect("seed API UI bindings");
    sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'schema-ui', 'index.html', $2, 'file', 'text/html'),
                ($1, 'schema-global', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(release)
    .bind(artifact)
    .execute(worker)
    .await
    .expect("seed static UI file");
    sqlx::query(
        "INSERT INTO gateways
         (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'browser-service', 'enabled', $4)",
    )
    .bind(managed_gateway)
    .bind(source_project)
    .bind(repository)
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed browser service gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id,
          release_agent_id, release_agent_key, handler_contract, exposure,
          parameters, secret_slots, mailbox_slots, service_loopback_port,
          service_readiness_path, service_health_path, service_log_capture_mode,
          normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'browser-service',
                 'http.service.v1', 'heph_authenticated', '{}'::jsonb,
                 ARRAY[]::text[], ARRAY[]::text[], 8080, '/ready', '/health',
                 'disabled', $7, $8)",
    )
    .bind(managed_revision)
    .bind(managed_gateway)
    .bind(source_project)
    .bind(repository)
    .bind(release)
    .bind(release_agent)
    .bind(vec![5_u8; 32])
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed browser service revision");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/service', ARRAY['GET', 'POST'])",
    )
    .bind(Uuid::new_v4())
    .bind(managed_revision)
    .bind(managed_gateway)
    .bind(source_project)
    .execute(worker)
    .await
    .expect("seed browser service routes");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(managed_gateway)
        .bind(managed_revision)
        .execute(worker)
        .await
        .expect("activate browser service revision");
    sqlx::query(
        "UPDATE releases SET state = 'published', published_at = now(),
                publication_actor_id = $2 WHERE id = $1",
    )
    .bind(release)
    .bind(actor)
    .execute(worker)
    .await
    .expect("publish release after descriptors");

    insert_canonical_session(worker, parent_session, actor, request_id, 20).await;
    insert_canonical_session(
        worker,
        outsider_parent_session,
        outsider,
        Uuid::new_v4(),
        20,
    )
    .await;
    seed_project_installation(
        worker,
        installation,
        generation,
        release,
        "schema-ui",
        actor,
        project,
    )
    .await;
    seed_global_installation(
        worker,
        global_installation,
        global_generation,
        release,
        "schema-global",
        actor,
        organization,
    )
    .await;
    seed_repository_installation(
        worker,
        repository_installation,
        repository_generation,
        release,
        "schema-repository",
        actor,
        source_project,
        repository,
    )
    .await;
    seed_project_installation(
        worker,
        other_installation,
        other_generation,
        release,
        "schema-ui-two",
        actor,
        project,
    )
    .await;
    seed_project_installation(
        worker,
        managed_installation,
        managed_generation,
        release,
        "schema-managed",
        actor,
        project,
    )
    .await;
    sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key,
          release_id, ui_key, gateway_id, gateway_revision_id,
          release_agent_id, gateway_name, method, route, exposure)
         VALUES
            ($1, $2, 'api', 'status', $3, 'schema-ui', $4, $5, $6,
             'browser-service', 'POST', '/service/api', 'heph_authenticated'),
            ($7, $8, 'managed_service', 'service', $3, 'schema-managed',
             $4, $5, $6, 'browser-service', 'GET', '/service',
             'heph_authenticated'),
            ($7, $8, 'api', 'status', $3, 'schema-managed', $4, $5, $6,
             'browser-service', 'POST', '/service/api', 'heph_authenticated')",
    )
    .bind(installation)
    .bind(generation)
    .bind(release)
    .bind(managed_gateway)
    .bind(managed_revision)
    .bind(release_agent)
    .bind(managed_installation)
    .bind(managed_generation)
    .execute(worker)
    .await
    .expect("seed static and managed/API installation bindings");

    Fixture {
        actor,
        outsider,
        organization,
        project,
        source_project,
        release,
        release_agent,
        other_organization,
        parent_session,
        outsider_parent_session,
        installation,
        other_installation,
        generation,
        other_generation,
        global_installation,
        global_generation,
        repository_installation,
        repository_generation,
        managed_installation,
        managed_generation,
        managed_gateway,
        managed_revision,
        route: "schema-ui",
    }
}

async fn insert_canonical_session(
    worker: &PgPool,
    session_id: Uuid,
    user_id: Uuid,
    request_id: Uuid,
    expiry_hours: i64,
) {
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + ($7::int * interval '1 hour'))",
    )
    .bind(session_id)
    .bind(digest(200))
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(digest(201))
    .bind(user_id)
    .bind(expiry_hours)
    .execute(worker)
    .await
    .expect("seed canonical human browser session");
}

async fn seed_project_installation(
    worker: &PgPool,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
    actor: Uuid,
    project: Uuid,
) {
    let mut tx = worker.begin().await.expect("begin installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, NULL, 'project', $3, 'enabled', $4, $5)",
    )
    .bind(installation_id)
    .bind(project)
    .bind(ui_key)
    .bind(generation_id)
    .bind(actor)
    .execute(&mut *tx)
    .await
    .expect("seed project UI installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'project')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed project UI generation");
    tx.commit().await.expect("commit project UI installation");
}

async fn seed_global_installation(
    worker: &PgPool,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
    actor: Uuid,
    organization: Uuid,
) {
    let mut tx = worker
        .begin()
        .await
        .expect("begin global installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', $3, 'enabled', $4, $5)",
    )
    .bind(installation_id)
    .bind(organization)
    .bind(ui_key)
    .bind(generation_id)
    .bind(actor)
    .execute(&mut *tx)
    .await
    .expect("seed global UI installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'global')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed global UI generation");
    tx.commit().await.expect("commit global UI installation");
}

// Keep the explicit installation fixture arguments aligned with the schema
// rows it creates; grouping them would hide the target-scope identity.
#[allow(clippy::too_many_arguments)]
async fn seed_repository_installation(
    worker: &PgPool,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
    actor: Uuid,
    project: Uuid,
    repository: Uuid,
) {
    let mut tx = worker
        .begin()
        .await
        .expect("begin repository installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, 'repository', $4, 'enabled', $5, $6)",
    )
    .bind(installation_id)
    .bind(project)
    .bind(repository)
    .bind(ui_key)
    .bind(generation_id)
    .bind(actor)
    .execute(&mut *tx)
    .await
    .expect("seed repository UI installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'repository')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed repository UI generation");
    tx.commit()
        .await
        .expect("commit repository UI installation");
}

fn digest(seed: u8) -> Vec<u8> {
    let mut value = vec![seed; 32];
    value[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    value
}

async fn insert_handoff(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    digest: Vec<u8>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 statement_timestamp(), statement_timestamp() + interval '60 seconds')",
    )
    .bind(id)
    .bind(digest)
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(installation)
    .bind(generation)
    .bind(organization)
    .bind(fixture.route)
    .execute(pool)
    .await
    .map(|_| id)
}

async fn insert_authenticated_child(
    pool: &PgPool,
    fixture: &Fixture,
    session_digest: Vec<u8>,
    child_expiry: &str,
) -> Uuid {
    let handoff = insert_handoff(
        pool,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(90),
    )
    .await
    .expect("insert authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin authentication child");
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                handoff.installation_id, handoff.generation_id, handoff.organization_id,
                handoff.route, handoff.issued_at, handoff.issued_at + $5::interval
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(child_id)
    .bind(session_digest)
    .bind(Uuid::new_v4())
    .bind(handoff)
    .bind(child_expiry)
    .execute(&mut *tx)
    .await
    .expect("insert authentication child");
    sqlx::query(
        "UPDATE ui_browser_handoffs
         SET consumed_at = statement_timestamp() WHERE id = $1",
    )
    .bind(handoff)
    .execute(&mut *tx)
    .await
    .expect("consume authentication handoff");
    tx.commit().await.expect("commit authentication child");
    child_id
}

async fn insert_authenticated_child_for_installation(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    session_digest: Vec<u8>,
) -> Uuid {
    let handoff = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route,
          issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 statement_timestamp(), statement_timestamp() + interval '60 seconds')",
    )
    .bind(handoff)
    .bind(digest(206))
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(installation)
    .bind(generation)
    .bind(fixture.organization)
    .bind(route)
    .execute(pool)
    .await
    .expect("insert unbound authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin unbound child");
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                handoff.installation_id, handoff.generation_id, handoff.organization_id,
                handoff.route, handoff.issued_at, handoff.issued_at + interval '1 hour'
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(child_id)
    .bind(session_digest)
    .bind(Uuid::new_v4())
    .bind(handoff)
    .execute(&mut *tx)
    .await
    .expect("insert unbound authentication child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume unbound authentication handoff");
    tx.commit().await.expect("commit unbound child");
    child_id
}

async fn insert_managed_authenticated_child(
    pool: &PgPool,
    fixture: &Fixture,
    session_secret: [u8; 32],
) -> Uuid {
    insert_managed_authenticated_child_with_expiry(pool, fixture, session_secret, "1 hour").await
}

async fn insert_managed_authenticated_child_with_expiry(
    pool: &PgPool,
    fixture: &Fixture,
    session_secret: [u8; 32],
    child_expiry: &str,
) -> Uuid {
    let handoff = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
         installation_id, generation_id, organization_id, route,
         issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'docs',
                 statement_timestamp(), statement_timestamp() + interval '60 seconds')",
    )
    .bind(handoff)
    .bind(digest(205))
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(fixture.managed_installation)
    .bind(fixture.managed_generation)
    .bind(fixture.organization)
    .execute(pool)
    .await
    .expect("insert managed authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin managed child");
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                handoff.installation_id, handoff.generation_id, handoff.organization_id,
                handoff.route, handoff.issued_at, handoff.issued_at + $5::interval
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(child_id)
    .bind(session_secret.to_vec())
    .bind(Uuid::new_v4())
    .bind(handoff)
    .bind(child_expiry)
    .execute(&mut *tx)
    .await
    .expect("insert managed authentication child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume managed authentication handoff");
    tx.commit().await.expect("commit managed child");
    child_id
}
