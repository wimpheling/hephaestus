//! Real-PostgreSQL matrix for UI browser persistence and application auth.

use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, time::Duration};
use tokio::time::timeout;
use uuid::Uuid;

use gateway_domain::{HttpMethod, RoutePath};
use identity_domain::{BrowserSessionId, RequestId, UserId};
use release_domain::{
    UiInstallationGenerationId, UiInstallationId,
    ui_browser::{UiBrowserHandoffSecret, UiBrowserRoute, UiBrowserSessionSecret},
};
use release_postgres::PgUiBrowserSessionStore;
use release_service::{
    AuthenticateUiBrowserSession, CreateUiBrowserHandoff, ExchangeUiBrowserHandoff,
    UiBrowserHandoffError, UiBrowserRequestRoute, UiBrowserSessionContext, UiBrowserSessionError,
};

const EXPECTED_MIGRATION: i64 = 96;

#[path = "ui_browser_schema/fixture.rs"]
mod fixture;
#[path = "ui_browser_schema/fixture_bindings.rs"]
mod fixture_bindings;
#[path = "ui_browser_schema/fixture_gateway.rs"]
mod fixture_gateway;
#[path = "ui_browser_schema/fixture_identity.rs"]
mod fixture_identity;
#[path = "ui_browser_schema/fixture_installations.rs"]
mod fixture_installations;
#[path = "ui_browser_schema/fixture_release.rs"]
mod fixture_release;
#[path = "ui_browser_schema/fixture_seed.rs"]
mod fixture_seed;

use fixture::insert_canonical_session;
use fixture::{Fixture, digest, scoped_secret, test_secret};
use fixture_seed::seed_fixture_reusing_installation_helpers;

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_browser_schema_matrix_enforces_bindings_lifecycle_timing_and_roles() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser schema: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0097");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker");
    assert!(max_migration >= EXPECTED_MIGRATION);

    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    assert_role(&worker, "hephaestus_worker", false, true).await;
    assert_role(&app, "hephaestus_app", false, false).await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;

    let partial_context = sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code,
             actor_id, installation_id, occurred_at)
         VALUES ($1, $2, 'handoff_issue', 'denied', 'not_attempted',
                 'unauthorized', $3, $4, statement_timestamp())",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.installation)
    .execute(&worker)
    .await;
    let partial_error = take_query_error(
        partial_context,
        "partial audit context unexpectedly succeeded",
    );
    assert_database_code(&partial_error, "23514", "partial verified target tuple");

    let audit_child = insert_audit_child(&worker, &fixture).await;
    let child_actor_mismatch = sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code,
             actor_id, organization_id, installation_id, generation_id,
             child_session_id, occurred_at)
         VALUES ($1, $2, 'handoff_exchange', 'allowed', 'succeeded', 'none',
                 $3, $4, $5, $6, $7, statement_timestamp())",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(fixture.outsider)
    .bind(fixture.organization)
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(audit_child)
    .execute(&worker)
    .await;
    let child_error = take_query_error(
        child_actor_mismatch,
        "child actor mismatch unexpectedly succeeded",
    );
    assert_database_code(&child_error, "23000", "child actor context");

    let valid_gateway_request = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code,
             actor_id, organization_id, installation_id, generation_id,
             gateway_id, gateway_revision_id, occurred_at)
         VALUES ($1, $2, 'managed', 'allowed', 'succeeded', 'none',
                 $3, $4, $5, $6, $7, $8, statement_timestamp())",
    )
    .bind(Uuid::new_v4())
    .bind(valid_gateway_request)
    .bind(fixture.actor)
    .bind(fixture.organization)
    .bind(fixture.managed_installation)
    .bind(fixture.managed_generation)
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .execute(&worker)
    .await
    .expect("generation gateway binding is accepted");
    let gateway_mismatch = sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code,
             actor_id, organization_id, installation_id, generation_id,
             gateway_id, gateway_revision_id, occurred_at)
         VALUES ($1, $2, 'managed', 'allowed', 'succeeded', 'none',
                 $3, $4, $5, $6, $7, $8, statement_timestamp())",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.organization)
    .bind(fixture.other_installation)
    .bind(fixture.other_generation)
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .execute(&worker)
    .await;
    let gateway_error = take_query_error(
        gateway_mismatch,
        "unbound gateway revision unexpectedly succeeded",
    );
    assert_database_code(&gateway_error, "23000", "historical gateway binding");

    assert_digest_constraints(&worker, &fixture).await;
    assert_wrong_actor_binding(&worker, &fixture).await;
    assert_wrong_organization_binding(&worker, &fixture).await;
    assert_wrong_installation_generation_binding(&worker, &fixture).await;
    assert_wrong_child_binding(&worker, &fixture).await;
    assert_initial_consumption_is_rejected(&worker, &fixture).await;
    assert_child_issue_interval_and_parent_cap(&worker, &fixture).await;
    assert_child_only_commit_rolls_back(&worker, &fixture).await;
    assert_consume_only_is_rejected(&worker, &fixture).await;
    assert_commit_and_consume_is_one_time(&worker, &fixture).await;
    assert_expiry_and_twelve_hour_cap(&worker, &fixture).await;
    assert_application_role_is_denied(&app, &fixture).await;

    println!(
        "REAL_UI_BROWSER_SCHEMA=1 migration={max_migration} digest=1 binding=1 initial_consumed=1 child_interval=1 transaction=1 one_time=1 expiry=1 role_denial=1"
    );
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_browser_application_authentication_is_generation_and_route_bound() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser authentication: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0097");
    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let child_digest = UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90))
        .digest()
        .as_bytes()
        .to_vec();
    let child_id =
        insert_authenticated_child(&worker, &fixture, child_digest.clone(), "1 hour").await;
    let store = PgUiBrowserSessionStore::new(worker.clone(), app.clone());

    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(fixture.outsider.to_string())
        .execute(&app)
        .await
        .expect("set spoofed application actor context");
    let valid_base = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await
        .expect("exact route base maps to entrypoint");
    assert_eq!(valid_base.session_id.as_uuid(), child_id);
    assert_eq!(valid_base.actor_id.as_uuid(), fixture.actor);
    assert_eq!(valid_base.organization_id.as_uuid(), fixture.organization);

    let valid_file = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui/index.html").expect("asset route"),
            },
        })
        .await
        .expect("published static asset authenticates");
    assert_eq!(valid_file.session_id.as_uuid(), child_id);

    let valid_static_api = authenticate_request(
        &store,
        fixture.generation,
        test_secret(fixture.actor, 90),
        UiBrowserRequestRoute::Api {
            route: RoutePath::parse("/service/api").expect("API route"),
            method: HttpMethod::Post,
        },
    )
    .await
    .expect("static UI API authenticates");
    assert_eq!(valid_static_api.session_id.as_uuid(), child_id);
    let global_child = insert_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.global_installation,
        fixture.global_generation,
        "schema-global",
        test_secret(fixture.actor, 95),
    )
    .await;
    let valid_global = authenticate_request(
        &store,
        fixture.global_generation,
        test_secret(fixture.actor, 95),
        UiBrowserRequestRoute::Static {
            route: UiBrowserRoute::parse("schema-global").expect("global route"),
        },
    )
    .await
    .expect("same-organization global UI authenticates");
    assert_eq!(valid_global.session_id.as_uuid(), global_child);
    assert_eq!(
        authenticate_request(
            &store,
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("API route"),
                method: HttpMethod::Get,
            },
        )
        .await,
        Err(UiBrowserSessionError::Unauthenticated)
    );

    let managed_child =
        insert_managed_authenticated_child(&worker, &fixture, test_secret(fixture.actor, 94)).await;
    let valid_managed = authenticate_request(
        &store,
        fixture.managed_generation,
        test_secret(fixture.actor, 94),
        UiBrowserRequestRoute::Managed {
            route: UiBrowserRoute::parse("schema-managed").expect("managed route"),
        },
    )
    .await
    .expect("managed UI authenticates");
    assert_eq!(valid_managed.session_id.as_uuid(), managed_child);
    let valid_managed_descendant = authenticate_request(
        &store,
        fixture.managed_generation,
        test_secret(fixture.actor, 94),
        UiBrowserRequestRoute::Managed {
            route: UiBrowserRoute::parse("schema-managed/child").expect("managed descendant"),
        },
    )
    .await
    .expect("managed UI descendant authenticates");
    assert_eq!(valid_managed_descendant.session_id.as_uuid(), managed_child);
    let valid_managed_api = authenticate_request(
        &store,
        fixture.managed_generation,
        test_secret(fixture.actor, 94),
        UiBrowserRequestRoute::Api {
            route: RoutePath::parse("/service/api").expect("managed API route"),
            method: HttpMethod::Post,
        },
    )
    .await
    .expect("managed UI API authenticates");
    assert_eq!(valid_managed_api.session_id.as_uuid(), managed_child);
    for request_route in [
        UiBrowserRequestRoute::Api {
            route: RoutePath::parse("/service/wrong").expect("wrong API route"),
            method: HttpMethod::Post,
        },
        UiBrowserRequestRoute::Api {
            route: RoutePath::parse("/service/api").expect("wrong method route"),
            method: HttpMethod::Get,
        },
    ] {
        assert_eq!(
            authenticate_request(
                &store,
                fixture.managed_generation,
                test_secret(fixture.actor, 94),
                request_route,
            )
            .await,
            Err(UiBrowserSessionError::Unauthenticated)
        );
    }

    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.managed_gateway)
        .execute(&worker)
        .await
        .expect("pause managed/API gateway");
    for (generation, secret, request_route) in [
        (
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("static API route"),
                method: HttpMethod::Post,
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Managed {
                route: UiBrowserRoute::parse("schema-managed").expect("managed route"),
            },
        ),
    ] {
        assert_eq!(
            authenticate_request(&store, generation, secret, request_route).await,
            Err(UiBrowserSessionError::Unauthenticated)
        );
    }
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.managed_gateway)
        .execute(&worker)
        .await
        .expect("restore managed/API gateway");

    let cutover_revision = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id,
          release_agent_id, release_agent_key, handler_contract, exposure,
          parameters, secret_slots, mailbox_slots, service_loopback_port,
          service_readiness_path, service_health_path, service_log_capture_mode,
          normalized_hash, created_by)
         SELECT $1, gateway_id, project_id, repository_id, release_id,
                release_agent_id, release_agent_key, handler_contract, exposure,
                parameters, secret_slots, mailbox_slots, service_loopback_port,
                service_readiness_path, service_health_path, service_log_capture_mode,
                $2, created_by
         FROM gateway_revisions WHERE id = $3",
    )
    .bind(cutover_revision)
    .bind(vec![6_u8; 32])
    .bind(fixture.managed_revision)
    .execute(&worker)
    .await
    .expect("seed gateway revision cutover");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         SELECT $1, $2, gateway_id, project_id, path, methods
         FROM gateway_routes WHERE gateway_revision_id = $3",
    )
    .bind(Uuid::new_v4())
    .bind(cutover_revision)
    .bind(fixture.managed_revision)
    .execute(&worker)
    .await
    .expect("seed cutover gateway route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(fixture.managed_gateway)
        .bind(cutover_revision)
        .execute(&worker)
        .await
        .expect("activate gateway revision cutover");
    for (generation, secret, request_route) in [
        (
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("static API route"),
                method: HttpMethod::Post,
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Managed {
                route: UiBrowserRoute::parse("schema-managed").expect("managed route"),
            },
        ),
    ] {
        assert_eq!(
            authenticate_request(&store, generation, secret, request_route).await,
            Err(UiBrowserSessionError::Unauthenticated)
        );
    }
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(fixture.managed_gateway)
        .bind(fixture.managed_revision)
        .execute(&worker)
        .await
        .expect("restore gateway revision");

    for route in ["schema-ui/missing.js", "other-ui/index.html"] {
        let denied = store
            .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
                session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
                expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
                request_route: UiBrowserRequestRoute::Static {
                    route: UiBrowserRoute::parse(route).expect("safe negative route"),
                },
            })
            .await;
        assert_eq!(denied, Err(UiBrowserSessionError::Unauthenticated));
    }
    let wrong_generation = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await;
    assert_eq!(
        wrong_generation,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    let wrong_kind = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Managed {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await;
    assert_eq!(wrong_kind, Err(UiBrowserSessionError::Unauthenticated));
    let wrong_method = sqlx::query(
        "SELECT session_id FROM authenticate_ui_browser_session(
             $1, $2, 'static', 'schema-ui', 'POST'
         )",
    )
    .bind(&child_digest)
    .bind(fixture.generation)
    .fetch_optional(&app)
    .await
    .expect("wrong method returns zero rows");
    assert!(wrong_method.is_none());

    let expired_digest = UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 91))
        .digest()
        .as_bytes()
        .to_vec();
    insert_authenticated_child(&worker, &fixture, expired_digest, "1 second").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let expired = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 91)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await;
    assert_eq!(expired, Err(UiBrowserSessionError::Unauthenticated));

    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("suspend account");
    let suspended = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await;
    assert_eq!(suspended, Err(UiBrowserSessionError::Unauthenticated));
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore account");

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
    .expect("insert replacement generation");
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(replacement_generation)
        .execute(&worker)
        .await
        .expect("move installation current generation");
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 90)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(fixture.generation)
        .execute(&worker)
        .await
        .expect("restore installation current generation");

    sqlx::query("UPDATE ui_installations SET lifecycle = 'disabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("disable installation");
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 90)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    sqlx::query("UPDATE ui_installations SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("restore installation lifecycle");

    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(fixture.project)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke target project maintainer");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke target organization membership");
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 90)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    sqlx::query("INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore target organization membership");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(fixture.project)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore target project maintainer");
    let initial_release_agent_use: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_use', 'release_agent', $2)")
            .bind(fixture.actor.to_string())
            .bind(fixture.release_agent.to_string())
            .fetch_one(&worker)
            .await
            .expect("check source release-agent use before revocation");
    assert_eq!(initial_release_agent_use, 1);

    // The release is published from source_project while the browser installs
    // live in the target project. Removing only source authority must deny
    // every source-backed binding and keep the explicit target read grant.
    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(fixture.source_project)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke source project maintainer");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke source organization membership");
    let target_read: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_read', 'project', $2)")
            .bind(fixture.actor.to_string())
            .bind(fixture.project.to_string())
            .fetch_one(&worker)
            .await
            .expect("check target project read after source revocation");
    assert_eq!(
        target_read, 1,
        "target project read survives source revocation"
    );
    let source_read: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_read', 'project', $2)")
            .bind(fixture.actor.to_string())
            .bind(fixture.source_project.to_string())
            .fetch_one(&worker)
            .await
            .expect("check source project read after revocation");
    assert_eq!(source_read, 0);
    let release_agent_use: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_use', 'release_agent', $2)")
            .bind(fixture.actor.to_string())
            .bind(fixture.release_agent.to_string())
            .fetch_one(&worker)
            .await
            .expect("check source release-agent use after revocation");
    assert_eq!(release_agent_use, 0);
    for (generation, secret, request_route) in [
        (
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("source-revoked static route"),
            },
        ),
        (
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("source-revoked static API route"),
                method: HttpMethod::Post,
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Managed {
                route: UiBrowserRoute::parse("schema-managed")
                    .expect("source-revoked managed route"),
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("source-revoked managed API route"),
                method: HttpMethod::Post,
            },
        ),
    ] {
        assert_eq!(
            authenticate_request(&store, generation, secret, request_route).await,
            Err(UiBrowserSessionError::Unauthenticated)
        );
    }
    sqlx::query("INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore source organization membership");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(fixture.source_project)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore source project maintainer");
    println!(
        "REAL_UI_BROWSER_AUTH_SOURCE_TARGET=1 target_read=1 source_read=0 release_agent_use=1_to_0 source_denials=4 cross_project=1 global_positive=1"
    );

    let expired_parent = Uuid::new_v4();
    insert_parent_with_times(
        &worker,
        expired_parent,
        fixture.actor,
        "0 seconds",
        "+2 seconds",
    )
    .await;
    insert_authenticated_child_for_parent(
        &worker,
        &fixture,
        expired_parent,
        test_secret(fixture.actor, 92),
        "1 second",
    )
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 92)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    let future_parent = Uuid::new_v4();
    insert_parent_with_times(
        &worker,
        future_parent,
        fixture.actor,
        "+1 minute",
        "+2 hours",
    )
    .await;
    insert_authenticated_child_for_parent(
        &worker,
        &fixture,
        future_parent,
        test_secret(fixture.actor, 93),
        "1 hour",
    )
    .await;
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 93)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );

    sqlx::query(
        "UPDATE releases SET state = 'revoked', revoked_at = statement_timestamp() WHERE id = $1",
    )
    .bind(fixture.release)
    .execute(&worker)
    .await
    .expect("revoke release for verifier denial");
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 90)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    for (generation, secret, request_route) in [
        (
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("revoked static API route"),
                method: HttpMethod::Post,
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Managed {
                route: UiBrowserRoute::parse("schema-managed").expect("revoked managed route"),
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("revoked managed API route"),
                method: HttpMethod::Post,
            },
        ),
    ] {
        assert_eq!(
            authenticate_request(&store, generation, secret, request_route).await,
            Err(UiBrowserSessionError::Unauthenticated)
        );
    }
    sqlx::query(
        "UPDATE human_browser_sessions
         SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
         WHERE id = $1",
    )
    .bind(fixture.parent_session)
    .execute(&worker)
    .await
    .expect("revoke parent session");
    let revoked = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await;
    assert_eq!(revoked, Err(UiBrowserSessionError::Unauthenticated));

    assert_application_auth_tables_are_denied(&app).await;
}

#[tokio::test]
#[serial]
async fn ui_browser_repository_git_authority_is_explicit_and_live() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser Git authority: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply UI Git migrations");
    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let repository_id: Uuid =
        sqlx::query_scalar("SELECT repository_id FROM ui_installations WHERE id = $1")
            .bind(fixture.repository_installation)
            .fetch_one(&worker)
            .await
            .expect("repository installation target");
    let secret = scoped_secret(fixture.actor, test_secret(fixture.actor, 97));
    insert_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.repository_installation,
        fixture.repository_generation,
        "schema-repository",
        test_secret(fixture.actor, 97),
    )
    .await;
    let digest = UiBrowserSessionSecret::from_bytes(secret)
        .digest()
        .as_bytes()
        .to_vec();
    let allowed: (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT actor_id, repository_id, access
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_one(&app)
    .await
    .expect("approved repository Git read");
    assert_eq!(allowed.0, fixture.actor);
    assert_eq!(allowed.1, repository_id);
    assert_eq!(allowed.2, "read");

    insert_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.no_git_repository_installation,
        fixture.no_git_repository_generation,
        "schema-repository-no-git",
        test_secret(fixture.actor, 99),
    )
    .await;
    let no_git_digest = UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 99))
        .digest()
        .as_bytes()
        .to_vec();
    let no_opt_in = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&no_git_digest)
    .bind(fixture.no_git_repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("no-opt-in denial query");
    assert!(no_opt_in.is_none());

    let wrong_repository = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(fixture.source_project)
    .fetch_optional(&app)
    .await
    .expect("wrong repository denial query");
    assert!(wrong_repository.is_none());

    insert_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.write_repository_installation,
        fixture.write_repository_generation,
        "schema-repository-write",
        test_secret(fixture.actor, 100),
    )
    .await;
    let write_digest = UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 100))
        .digest()
        .as_bytes()
        .to_vec();
    let write_allowed = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'write')",
    )
    .bind(&write_digest)
    .bind(fixture.write_repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("approved repository Git write query");
    assert_eq!(write_allowed, Some(repository_id));

    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke write-level organization grant");
    sqlx::query(
        "DELETE FROM project_maintainers
         WHERE user_id = $1 AND project_id IN ($2, $3)",
    )
    .bind(fixture.actor)
    .bind(fixture.project)
    .bind(fixture.source_project)
    .execute(&worker)
    .await
    .expect("revoke direct project write grants");
    sqlx::query("DELETE FROM repository_managers WHERE repository_id = $1 AND user_id = $2")
        .bind(repository_id)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke direct repository write grant");
    let write_without_grant = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'write')",
    )
    .bind(&write_digest)
    .bind(fixture.write_repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("revoked write grant denial query");
    assert!(write_without_grant.is_none());
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner') ON CONFLICT (organization_id, user_id)
         DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("restore write-level organization grant");
    let write_without_declaration = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'write')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("read-only write denial query");
    assert!(write_without_declaration.is_none());

    let stale_generation = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.other_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("stale generation denial query");
    assert!(stale_generation.is_none());

    let unknown_child = UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 98))
        .digest()
        .as_bytes()
        .to_vec();
    let revoked_child = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&unknown_child)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("unknown child denial query");
    assert!(revoked_child.is_none());

    let expired_secret = test_secret(fixture.actor, 101);
    insert_expired_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.repository_installation,
        fixture.repository_generation,
        "schema-repository",
        expired_secret,
    )
    .await;
    let expired_digest = UiBrowserSessionSecret::from_bytes(expired_secret)
        .digest()
        .as_bytes()
        .to_vec();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let expired_child = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&expired_digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("expired child denial query");
    assert!(expired_child.is_none());

    sqlx::query("UPDATE ui_installations SET lifecycle = 'disabled' WHERE id = $1")
        .bind(fixture.repository_installation)
        .execute(&worker)
        .await
        .expect("disable repository UI installation");
    let disabled_installation = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("disabled installation denial query");
    assert!(disabled_installation.is_none());
    sqlx::query("UPDATE ui_installations SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.repository_installation)
        .execute(&worker)
        .await
        .expect("restore repository UI installation");

    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke live repository Git grants");
    let revoked_grants = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("revoked grants denial query");
    assert!(revoked_grants.is_none());
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner') ON CONFLICT (organization_id, user_id)
         DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("restore live repository Git grants");

    // Release revocation is intentionally terminal, so keep this mutation as
    // the final assertion in the disposable fixture.
    sqlx::query(
        "UPDATE releases
         SET state = 'revoked', revoked_at = statement_timestamp()
         WHERE id = $1",
    )
    .bind(fixture.release)
    .execute(&worker)
    .await
    .expect("revoke published release");
    let revoked_release = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("revoked release denial query");
    assert!(revoked_release.is_none());
    sqlx::query(
        "UPDATE human_browser_sessions
         SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
         WHERE id = $1",
    )
    .bind(fixture.parent_session)
    .execute(&worker)
    .await
    .expect("revoke repository parent session");
    let revoked_parent = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("revoked parent denial query");
    assert!(revoked_parent.is_none());
    println!(
        "REAL_UI_BROWSER_REPOSITORY_GIT_AUTHORITY=1 opt_in=1 no_opt_in=1 wrong_repository=1 read_only_write_denied=1 write_allowed=1 write_grant_revoked=1 stale_generation=1 child_expired=1 parent_revoked=1 installation_disabled=1 release_revoked=1 grants_revoked=1"
    );
}

async fn authenticate_static(
    store: &PgUiBrowserSessionStore,
    fixture: &Fixture,
    secret: [u8; 32],
) -> Result<UiBrowserSessionContext, UiBrowserSessionError> {
    authenticate_request(
        store,
        fixture.generation,
        secret,
        UiBrowserRequestRoute::Static {
            route: UiBrowserRoute::parse("schema-ui").expect("route base"),
        },
    )
    .await
}

async fn authenticate_request(
    store: &PgUiBrowserSessionStore,
    generation: Uuid,
    secret: [u8; 32],
    request_route: UiBrowserRequestRoute,
) -> Result<UiBrowserSessionContext, UiBrowserSessionError> {
    store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(secret),
            expected_generation_id: UiInstallationGenerationId::from_uuid(generation),
            request_route,
        })
        .await
}

async fn assert_digest_constraints(worker: &PgPool, fixture: &Fixture) {
    let handoff = Uuid::new_v4();
    let bad_lengths = [vec![0_u8; 31], vec![0_u8; 33]];
    for digest in bad_lengths {
        assert_rejected_code(
            sqlx::query(
                "INSERT INTO ui_browser_handoffs
                 (id, handoff_digest, request_id, actor_id, parent_session_id,
                  installation_id, generation_id, organization_id, route,
                  issued_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                         statement_timestamp(), statement_timestamp() + interval '60 seconds')",
            )
            .bind(handoff)
            .bind(&digest)
            .bind(Uuid::new_v4())
            .bind(fixture.actor)
            .bind(fixture.parent_session)
            .bind(fixture.installation)
            .bind(fixture.generation)
            .bind(fixture.organization)
            .bind(fixture.route)
            .execute(worker),
            "23514",
            "handoff digest must be exactly 32 bytes",
        )
        .await;
    }
    let absent: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1")
            .bind(vec![0_u8; 32])
            .fetch_optional(worker)
            .await
            .expect("worker can perform a digest lookup for this matrix");
    assert!(
        absent.is_none(),
        "an unrelated digest must not authenticate"
    );
}

async fn assert_wrong_actor_binding(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        sqlx::query(
            "INSERT INTO ui_browser_handoffs
             (id, handoff_digest, request_id, actor_id, parent_session_id,
              installation_id, generation_id, organization_id, route,
              issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     statement_timestamp(), statement_timestamp() + interval '60 seconds')",
        )
        .bind(Uuid::new_v4())
        .bind(digest(1))
        .bind(Uuid::new_v4())
        .bind(fixture.outsider)
        .bind(fixture.parent_session)
        .bind(fixture.installation)
        .bind(fixture.generation)
        .bind(fixture.organization)
        .bind(fixture.route)
        .execute(worker),
        "23000",
        "actor must own the canonical parent session",
    )
    .await;
}

async fn assert_wrong_organization_binding(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        insert_handoff(
            worker,
            fixture,
            fixture.other_organization,
            fixture.installation,
            fixture.generation,
            digest(2),
        ),
        "23000",
        "organization must be derived from the installation target",
    )
    .await;
}

async fn assert_wrong_installation_generation_binding(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        insert_handoff(
            worker,
            fixture,
            fixture.organization,
            fixture.installation,
            fixture.other_generation,
            digest(3),
        ),
        "23503",
        "generation and installation must satisfy the composite FK",
    )
    .await;
    assert_rejected_code(
        insert_handoff(
            worker,
            fixture,
            fixture.organization,
            fixture.other_installation,
            fixture.generation,
            digest(4),
        ),
        "23503",
        "generation must belong to the selected installation",
    )
    .await;
}

async fn assert_wrong_child_binding(worker: &PgPool, fixture: &Fixture) {
    let cases = [
        (
            fixture.outsider_parent_session,
            fixture.installation,
            fixture.generation,
            fixture.organization,
            "parent actor/session binding",
        ),
        (
            fixture.parent_session,
            fixture.installation,
            fixture.generation,
            fixture.other_organization,
            "child organization binding",
        ),
        (
            fixture.parent_session,
            fixture.other_installation,
            fixture.other_generation,
            fixture.organization,
            "child installation/generation binding",
        ),
    ];
    for (parent_session, installation, generation, organization, reason) in cases {
        let handoff = insert_handoff(
            worker,
            fixture,
            fixture.organization,
            fixture.installation,
            fixture.generation,
            digest(20),
        )
        .await
        .expect("insert handoff for child binding case");
        assert_rejected_code(
            insert_child_with_binding(
                worker,
                fixture,
                handoff,
                parent_session,
                installation,
                generation,
                organization,
                digest(21),
            ),
            "23000",
            reason,
        )
        .await;
    }
}

async fn assert_initial_consumption_is_rejected(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        sqlx::query(
            "INSERT INTO ui_browser_handoffs
             (id, handoff_digest, request_id, actor_id, parent_session_id,
              installation_id, generation_id, organization_id, route,
              issued_at, expires_at, consumed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     statement_timestamp(), statement_timestamp() + interval '60 seconds',
                     statement_timestamp())",
        )
        .bind(Uuid::new_v4())
        .bind(digest(5))
        .bind(Uuid::new_v4())
        .bind(fixture.actor)
        .bind(fixture.parent_session)
        .bind(fixture.installation)
        .bind(fixture.generation)
        .bind(fixture.organization)
        .bind(fixture.route)
        .execute(worker),
        "23000",
        "consumed_at is a transition and cannot be supplied on INSERT",
    )
    .await;
}

async fn assert_child_issue_interval_and_parent_cap(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(6),
    )
    .await
    .expect("insert valid handoff");
    assert_rejected_code(
        insert_child(worker, fixture, handoff, "60 seconds", "60 seconds"),
        "23000",
        "child issue at handoff expiry must fail",
    )
    .await;
    let before_child_handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(15),
    )
    .await
    .expect("insert handoff for child-issue ordering");
    let mut tx = worker
        .begin()
        .await
        .expect("begin child-issue ordering transaction");
    insert_child_tx(
        &mut tx,
        fixture,
        before_child_handoff,
        "30 seconds",
        "1 hour",
    )
    .await
    .expect("insert future-issued child");
    let before_child_error = sqlx::query(
        "UPDATE ui_browser_handoffs
         SET consumed_at = issued_at + interval '1 second' WHERE id = $1",
    )
    .bind(before_child_handoff)
    .execute(&mut *tx)
    .await
    .expect_err("consumption before child issue should fail");
    assert_database_code(
        &before_child_error,
        "23000",
        "consumption before child issue",
    );
    tx.rollback()
        .await
        .expect("rollback child-issue ordering rejection");
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(7),
    )
    .await
    .expect("insert second valid handoff");
    assert_rejected_code(
        insert_child(worker, fixture, handoff, "1 second", "13 hours"),
        "23514",
        "child expiry may not exceed the twelve-hour cap or parent expiry",
    )
    .await;
}

async fn assert_child_only_commit_rolls_back(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(8),
    )
    .await
    .expect("insert valid handoff");
    let mut tx = worker.begin().await.expect("begin worker transaction");
    insert_child_tx(&mut tx, fixture, handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before deferred rollback");
    let commit_error = tx
        .commit()
        .await
        .expect_err("deferred consumed trigger must reject child-only commit");
    assert_database_code(&commit_error, "23000", "child-only commit");
    let consumed: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(handoff)
            .fetch_one(worker)
            .await
            .expect("handoff remains after child-only rollback");
    assert!(consumed.is_none());
    let child_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(handoff)
            .fetch_one(worker)
            .await
            .expect("count child rows after rollback");
    assert_eq!(
        child_count, 0,
        "child-only rollback must leave no child row"
    );
}

async fn assert_consume_only_is_rejected(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(9),
    )
    .await
    .expect("insert valid handoff");
    assert_rejected_code(
        sqlx::query(
            "UPDATE ui_browser_handoffs
             SET consumed_at = statement_timestamp() WHERE id = $1",
        )
        .bind(handoff)
        .execute(worker),
        "23000",
        "handoff cannot be consumed without a child",
    )
    .await;
}

async fn assert_commit_and_consume_is_one_time(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(10),
    )
    .await
    .expect("insert valid handoff");
    let mut tx = worker.begin().await.expect("begin worker transaction");
    insert_child_tx(&mut tx, fixture, handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before one-time consume");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume matching child in same transaction");
    tx.commit()
        .await
        .expect("commit child and consume atomically");
    assert_rejected_code(
        insert_child(worker, fixture, handoff, "0 seconds", "1 hour"),
        "23000",
        "unique handoff_id prevents a second child",
    )
    .await;
    assert_rejected_code(
        sqlx::query(
            "UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1",
        )
        .bind(handoff)
        .execute(worker),
        "23000",
        "a consumed handoff cannot be reconsumed",
    )
    .await;
}

async fn assert_expiry_and_twelve_hour_cap(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        insert_handoff_with_times(worker, fixture, digest(11), "-61 seconds", "0 seconds"),
        "23514",
        "handoff lifetime must be exactly sixty seconds",
    )
    .await;
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(12),
    )
    .await
    .expect("insert valid handoff");
    let mut tx = worker.begin().await.expect("begin worker transaction");
    insert_child_tx(&mut tx, fixture, handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before expiry assertion");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume child before expiry assertion");
    tx.commit().await.expect("commit valid child and consume");
    let expired_handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(14),
    )
    .await
    .expect("insert second handoff for expiry timestamp");
    let mut tx = worker.begin().await.expect("begin expiry transaction");
    insert_child_tx(&mut tx, fixture, expired_handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before expiry rejection");
    let expiry_error = sqlx::query(
        "UPDATE ui_browser_handoffs
         SET consumed_at = issued_at + interval '60 seconds' WHERE id = $1",
    )
    .bind(expired_handoff)
    .execute(&mut *tx)
    .await
    .expect_err("consumption at expiry should fail before commit");
    assert_database_code(&expiry_error, "23000", "consumption at expiry");
    tx.rollback().await.expect("rollback expiry rejection");
}

async fn assert_application_role_is_denied(app: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        sqlx::query("SELECT * FROM ui_browser_handoffs").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("SELECT * FROM ui_browser_sessions").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("INSERT INTO ui_browser_handoffs DEFAULT VALUES").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("INSERT INTO ui_browser_sessions DEFAULT VALUES").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = now()").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("DELETE FROM ui_browser_handoffs").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("DELETE FROM ui_browser_sessions").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    let _ = fixture;
}

async fn assert_application_auth_tables_are_denied(app: &PgPool) {
    match sqlx::query("SELECT * FROM ui_browser_sessions")
        .fetch_all(app)
        .await
    {
        Ok(rows) => assert!(rows.is_empty(), "application role read session rows"),
        Err(error) => assert_database_code(&error, "42501", "application verifier table access"),
    }
    match sqlx::query("SELECT * FROM ui_browser_handoffs")
        .fetch_all(app)
        .await
    {
        Ok(rows) => assert!(rows.is_empty(), "application role read handoff rows"),
        Err(error) => assert_database_code(&error, "42501", "application verifier table access"),
    }
    match sqlx::query("SELECT * FROM human_browser_sessions")
        .fetch_all(app)
        .await
    {
        Ok(rows) => assert!(rows.is_empty(), "application role read parent session rows"),
        Err(error) => assert_database_code(&error, "42501", "application verifier table access"),
    }
    match sqlx::query("SELECT * FROM ui_installation_bindings")
        .fetch_all(app)
        .await
    {
        Ok(rows) => assert!(
            rows.is_empty(),
            "application role read installation bindings"
        ),
        Err(error) => assert_database_code(&error, "42501", "application verifier table access"),
    }
}

// The following helpers deliberately centralize the statement-time expressions
// so every matrix case remains comparable with the draft trigger checks.
async fn insert_handoff(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    digest: Vec<u8>,
) -> Result<Uuid, sqlx::Error> {
    insert_handoff_with_times_for(
        pool,
        fixture,
        organization,
        installation,
        generation,
        digest,
        "0 seconds",
        "60 seconds",
    )
    .await
}

async fn insert_handoff_with_times(
    pool: &PgPool,
    fixture: &Fixture,
    digest: Vec<u8>,
    issued_at: &str,
    expires_at: &str,
) -> Result<Uuid, sqlx::Error> {
    insert_handoff_with_times_for(
        pool,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest,
        issued_at,
        expires_at,
    )
    .await
}

// The matrix keeps each persisted binding dimension visible at the call site.
#[allow(clippy::too_many_arguments)]
async fn insert_handoff_with_times_for(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    digest: Vec<u8>,
    issued_at: &str,
    expires_at: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 statement_timestamp() + $10::interval,
                 statement_timestamp() + $11::interval)",
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
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await
    .map(|_| id)
}

async fn insert_child(
    pool: &PgPool,
    fixture: &Fixture,
    handoff: Uuid,
    issued_at: &str,
    expires_at: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    insert_child_tx(&mut tx, fixture, handoff, issued_at, expires_at).await?;
    tx.commit().await
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

async fn insert_audit_child(pool: &PgPool, fixture: &Fixture) -> Uuid {
    let handoff_digest = UiBrowserHandoffSecret::random()
        .digest()
        .as_bytes()
        .to_vec();
    let handoff = insert_handoff(
        pool,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        handoff_digest,
    )
    .await
    .expect("insert audit child handoff");
    let child_id = Uuid::new_v4();
    let mut transaction = pool.begin().await.expect("begin audit child transaction");
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
    .bind(
        UiBrowserSessionSecret::random()
            .digest()
            .as_bytes()
            .to_vec(),
    )
    .bind(Uuid::new_v4())
    .bind(handoff)
    .execute(&mut *transaction)
    .await
    .expect("insert audit child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *transaction)
        .await
        .expect("consume audit child handoff");
    transaction.commit().await.expect("commit audit child");
    child_id
}

async fn insert_authenticated_child_for_installation(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    session_secret: [u8; 32],
) -> Uuid {
    insert_authenticated_child_for_installation_with_expiry(
        pool,
        fixture,
        installation,
        generation,
        route,
        session_secret,
        false,
    )
    .await
}

async fn insert_expired_authenticated_child_for_installation(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    session_secret: [u8; 32],
) -> Uuid {
    insert_authenticated_child_for_installation_with_expiry(
        pool,
        fixture,
        installation,
        generation,
        route,
        session_secret,
        true,
    )
    .await
}

async fn insert_authenticated_child_for_installation_with_expiry(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    session_secret: [u8; 32],
    expired: bool,
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
    .expect("insert installation-bound authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin installation-bound child");
    let child_insert = if expired {
        sqlx::query(
            "INSERT INTO ui_browser_sessions
             (id, session_digest, request_id, handoff_id, parent_session_id,
              installation_id, generation_id, organization_id, route, issued_at, expires_at)
             SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                    handoff.installation_id, handoff.generation_id, handoff.organization_id,
                    handoff.route, handoff.issued_at,
                    handoff.issued_at + interval '1 second'
             FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
        )
    } else {
        sqlx::query(
            "INSERT INTO ui_browser_sessions
             (id, session_digest, request_id, handoff_id, parent_session_id,
              installation_id, generation_id, organization_id, route, issued_at, expires_at)
             SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                    handoff.installation_id, handoff.generation_id, handoff.organization_id,
                    handoff.route, handoff.issued_at, handoff.issued_at + interval '1 hour'
             FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
        )
    };
    child_insert
        .bind(child_id)
        .bind(
            UiBrowserSessionSecret::from_bytes(scoped_secret(fixture.actor, session_secret))
                .digest()
                .as_bytes()
                .to_vec(),
        )
        .bind(Uuid::new_v4())
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("insert installation-bound authentication child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume installation-bound authentication handoff");
    tx.commit()
        .await
        .expect("commit installation-bound authentication child");
    child_id
}

async fn insert_parent_with_times(
    pool: &PgPool,
    parent_id: Uuid,
    actor: Uuid,
    issued_at: &str,
    expires_at: &str,
) {
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6,
                 statement_timestamp() + $7::interval,
                 statement_timestamp() + $8::interval)",
    )
    .bind(parent_id)
    .bind(digest(202))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(203))
    .bind(actor)
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("insert parent with explicit timestamps");
}

async fn insert_authenticated_child_for_parent(
    pool: &PgPool,
    fixture: &Fixture,
    parent_session: Uuid,
    session_secret: [u8; 32],
    child_expiry: &str,
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
    .bind(digest(204))
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(parent_session)
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.organization)
    .bind(fixture.route)
    .execute(pool)
    .await
    .expect("insert parent-bound authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin parent-bound child");
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
    .bind(
        UiBrowserSessionSecret::from_bytes(scoped_secret(fixture.actor, session_secret))
            .digest()
            .as_bytes()
            .to_vec(),
    )
    .bind(Uuid::new_v4())
    .bind(handoff)
    .bind(child_expiry)
    .execute(&mut *tx)
    .await
    .expect("insert parent-bound authentication child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume parent-bound handoff");
    tx.commit().await.expect("commit parent-bound child");
    child_id
}

async fn insert_managed_authenticated_child(
    pool: &PgPool,
    fixture: &Fixture,
    session_secret: [u8; 32],
) -> Uuid {
    let handoff = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route,
          issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'schema-managed',
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
                handoff.route, handoff.issued_at, handoff.issued_at + interval '1 hour'
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(child_id)
    .bind(
        UiBrowserSessionSecret::from_bytes(scoped_secret(fixture.actor, session_secret))
            .digest()
            .as_bytes()
            .to_vec(),
    )
    .bind(Uuid::new_v4())
    .bind(handoff)
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

// The negative cases intentionally override each durable binding independently.
#[allow(clippy::too_many_arguments)]
async fn insert_child_with_binding(
    pool: &PgPool,
    fixture: &Fixture,
    handoff: Uuid,
    parent_session: Uuid,
    installation: Uuid,
    generation: Uuid,
    organization: Uuid,
    session_digest: Vec<u8>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, $4, $5, $6, $7, $8,
                handoff.issued_at, handoff.issued_at + interval '1 hour'
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $9",
    )
    .bind(Uuid::new_v4())
    .bind(session_digest)
    .bind(Uuid::new_v4())
    .bind(parent_session)
    .bind(installation)
    .bind(generation)
    .bind(organization)
    .bind(fixture.route)
    .bind(handoff)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

async fn insert_child_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    _fixture: &Fixture,
    handoff: Uuid,
    issued_at: &str,
    expires_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                handoff.installation_id, handoff.generation_id, handoff.organization_id,
                handoff.route, handoff.issued_at + $5::interval,
                handoff.issued_at + $6::interval
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(Uuid::new_v4())
    .bind(digest(13))
    .bind(Uuid::new_v4())
    .bind(handoff)
    .bind(issued_at)
    .bind(expires_at)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}

async fn assert_rejected_code<F, T>(operation: F, expected_code: &str, reason: &str)
where
    F: std::future::Future<Output = Result<T, sqlx::Error>>,
{
    let result = timeout(Duration::from_secs(5), operation)
        .await
        .expect("schema operation did not finish");
    let error = match result {
        Ok(_) => panic!("schema operation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_database_code(&error, expected_code, reason);
}

fn assert_database_code(error: &sqlx::Error, expected_code: &str, reason: &str) {
    let actual_code = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code);
    assert_eq!(
        actual_code.as_deref(),
        Some(expected_code),
        "unexpected SQLSTATE for {reason}"
    );
}

fn take_query_error<T>(result: Result<T, sqlx::Error>, message: &str) -> sqlx::Error {
    match result {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}

// These helpers match the existing release schema tests. The test URL's login
// role must be allowed to SET ROLE to each restricted disposable-db role.
async fn role_pool(database_url: &str, role: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect({
            let role = role.to_owned();
            move |connection, _metadata| {
                let role = role.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('role', $1, false)")
                        .bind(role.clone())
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('application_name', $1, false)")
                        .bind(format!("ui-browser-{role}"))
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
                        .bind(Uuid::nil().to_string())
                        .execute(&mut *connection)
                        .await
                        .map(|_| ())
                })
            }
        })
        .connect(database_url)
        .await
        .expect("connect restricted PostgreSQL role")
}

async fn wait_for_named_lock_waiter(admin: &PgPool, application_name: &str, blocker_pid: i32) {
    for _ in 0..200 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE application_name = $1
               AND wait_event_type = 'Lock'
               AND $2 = ANY(pg_blocking_pids(pid))",
        )
        .bind(application_name)
        .bind(blocker_pid)
        .fetch_one(admin)
        .await
        .expect("inspect issuance lock waiter");
        if waiting > 0 {
            println!(
                "REAL_UI_BROWSER_ISSUE_LOCK_BARRIER=1 application_name={application_name} blocker_pid={blocker_pid}"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for issuance lock waiter {application_name}");
}

async fn wait_for_named_exchange_lock_waiter(
    admin: &PgPool,
    application_name: &str,
    blocker_pid: i32,
) {
    for _ in 0..1000 {
        let waiting: i64 = sqlx::query_scalar(
            "WITH RECURSIVE blocker_chain(pid, blocking_pid, depth) AS (
                 SELECT activity.pid, unnest(pg_blocking_pids(activity.pid)), 0
                 FROM pg_stat_activity AS activity
                 WHERE activity.application_name = $1
                   AND activity.wait_event_type = 'Lock'
                 UNION ALL
                 SELECT blocker_chain.pid,
                        unnest(pg_blocking_pids(blocker_chain.blocking_pid)),
                        blocker_chain.depth + 1
                 FROM blocker_chain
                 WHERE blocker_chain.depth < 4
             )
             SELECT count(*) FROM blocker_chain WHERE blocking_pid = $2",
        )
        .bind(application_name)
        .bind(blocker_pid)
        .fetch_one(admin)
        .await
        .expect("inspect exchange lock waiter");
        if waiting > 0 {
            println!(
                "REAL_UI_BROWSER_EXCHANGE_LOCK_BARRIER=1 application_name={application_name} blocker_pid={blocker_pid}"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for exchange lock waiter {application_name}");
}

async fn assert_role(pool: &PgPool, expected: &str, superuser: bool, bypass_rls: bool) {
    let row: (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, rolsuper, rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await
    .expect("read PostgreSQL role identity");
    assert_eq!(row, (expected.to_owned(), superuser, bypass_rls));
}

async fn assert_audit_row(
    pool: &PgPool,
    request_id: RequestId,
    surface: &str,
    decision: &str,
    outcome: &str,
    reason: &str,
) {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT surface, decision, outcome, reason_code
         FROM ui_request_audit_events
         WHERE request_id = $1 AND surface = $2",
    )
    .bind(request_id.as_uuid())
    .bind(surface)
    .fetch_all(pool)
    .await
    .expect("read UI request audit row");
    assert_eq!(rows.len(), 1, "expected one {surface} audit row");
    assert_eq!(
        rows[0],
        (
            surface.into(),
            decision.into(),
            outcome.into(),
            reason.into()
        )
    );
}

// This assertion keeps every verified foreign-key context field explicit so
// an audit row cannot silently omit one relationship.
#[allow(clippy::too_many_arguments)]
async fn assert_audit_context(
    pool: &PgPool,
    request_id: RequestId,
    surface: &str,
    actor: Uuid,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    child_session: Option<Uuid>,
) {
    type AuditContext = (
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
    );
    let context: AuditContext = sqlx::query_as(
        "SELECT actor_id, organization_id, installation_id, generation_id,
                child_session_id, gateway_id, gateway_revision_id
         FROM ui_request_audit_events
         WHERE request_id = $1 AND surface = $2",
    )
    .bind(request_id.as_uuid())
    .bind(surface)
    .fetch_one(pool)
    .await
    .expect("read UI request audit context");
    assert_eq!(
        context,
        (
            Some(actor),
            Some(organization),
            Some(installation),
            Some(generation),
            child_session,
            None,
            None,
        ),
        "verified context for {surface} audit row"
    );
}

async fn assert_audit_actor_only(pool: &PgPool, request_id: RequestId, actor: Uuid) {
    type AuditSubjectContext = (
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
    );
    let context: AuditSubjectContext = sqlx::query_as(
        "SELECT actor_id, organization_id, installation_id, generation_id,
                child_session_id
         FROM ui_request_audit_events
         WHERE request_id = $1 AND surface = 'handoff_issue'",
    )
    .bind(request_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("read actor-only UI request audit context");
    assert_eq!(context, (Some(actor), None, None, None, None));
}

async fn assert_audit_anonymous(pool: &PgPool, request_id: RequestId, surface: &str) {
    type AuditSubjectContext = (
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
    );
    let context: AuditSubjectContext = sqlx::query_as(
        "SELECT actor_id, organization_id, installation_id, generation_id,
                child_session_id
         FROM ui_request_audit_events
         WHERE request_id = $1 AND surface = $2",
    )
    .bind(request_id.as_uuid())
    .bind(surface)
    .fetch_one(pool)
    .await
    .expect("read anonymous UI request audit context");
    assert_eq!(context, (None, None, None, None, None));
}

/// Exercises the first adapter slice against the complete 0089 fixture. The
/// schema matrix above remains the owner of SQLSTATE and lifecycle checks.
#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_browser_issue_binds_current_authority_and_fresh_expiry() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser issue: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0097");
    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let store = PgUiBrowserSessionStore::new(worker.clone(), app);
    let barrier_app = role_pool(&database_url, "hephaestus_app").await;
    let actor = UserId::from_uuid(fixture.actor);
    let parent = BrowserSessionId::from_uuid(fixture.parent_session);
    let installation = UiInstallationId::from_uuid(fixture.installation);
    let generation = UiInstallationGenerationId::from_uuid(fixture.generation);
    let route = UiBrowserRoute::parse("schema-ui").expect("published route base");
    let request_id = RequestId::new();
    let secret = UiBrowserHandoffSecret::random();
    let expected_digest = secret.digest().as_bytes();
    let created = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret,
        })
        .await
        .expect("active actor may issue for current generation");
    assert_eq!(created.organization_id.as_uuid(), fixture.organization);
    assert_eq!(created.route, route);
    assert_audit_row(
        &bootstrap,
        request_id,
        "handoff_issue",
        "allowed",
        "succeeded",
        "none",
    )
    .await;
    assert_audit_row(
        &bootstrap,
        request_id,
        "embed",
        "allowed",
        "succeeded",
        "none",
    )
    .await;
    assert_audit_context(
        &bootstrap,
        request_id,
        "handoff_issue",
        fixture.actor,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        None,
    )
    .await;
    assert_audit_context(
        &bootstrap,
        request_id,
        "embed",
        fixture.actor,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        None,
    )
    .await;

    let full_page_request_id = RequestId::new();
    store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: full_page_request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("full-page route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await
        .expect("full-page UI may issue a handoff");
    assert_audit_row(
        &bootstrap,
        full_page_request_id,
        "handoff_issue",
        "allowed",
        "succeeded",
        "none",
    )
    .await;
    let full_page_embeds: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_request_audit_events
         WHERE request_id = $1 AND surface = 'embed'",
    )
    .bind(full_page_request_id.as_uuid())
    .fetch_one(&bootstrap)
    .await
    .expect("count full-page embed audit rows");
    assert_eq!(
        full_page_embeds, 0,
        "full-page UI must not emit embed audit"
    );

    let handoff_count_before_audit_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(&bootstrap)
            .await
            .expect("count handoffs before audit failure");
    sqlx::query("REVOKE INSERT ON public.ui_request_audit_events FROM hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("revoke audit insert for atomicity test");
    let audit_failure_request_id = RequestId::new();
    let audit_failure = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: audit_failure_request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(audit_failure, Err(UiBrowserHandoffError::Unavailable));
    sqlx::query("GRANT INSERT ON public.ui_request_audit_events TO hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("restore audit insert after atomicity test");
    let handoff_count_after_audit_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(&bootstrap)
            .await
            .expect("count handoffs after audit failure");
    assert_eq!(
        handoff_count_after_audit_failure, handoff_count_before_audit_failure,
        "audit append failure rolled back the handoff mutation"
    );
    let audit_failure_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_request_audit_events WHERE request_id = $1")
            .bind(audit_failure_request_id.as_uuid())
            .fetch_one(&bootstrap)
            .await
            .expect("count audit failure rows");
    assert_eq!(audit_failure_rows, 0);

    let stored: (
        Vec<u8>,
        Uuid,
        Uuid,
        Uuid,
        Uuid,
        String,
        time::OffsetDateTime,
        time::OffsetDateTime,
    ) = sqlx::query_as(
        "SELECT handoff_digest, request_id, actor_id, parent_session_id,
                    organization_id, route, issued_at, expires_at
             FROM ui_browser_handoffs WHERE id = $1",
    )
    .bind(created.handoff_id.as_uuid())
    .fetch_one(&worker)
    .await
    .expect("read issued handoff metadata");
    assert_eq!(
        stored.0, expected_digest,
        "stored digest matches the secret"
    );
    assert_eq!(stored.1, request_id.as_uuid());
    assert_eq!(stored.2, fixture.actor);
    assert_eq!(stored.3, fixture.parent_session);
    assert_eq!(stored.4, fixture.organization);
    assert_eq!(stored.5, "schema-ui");
    assert_eq!(stored.7 - stored.6, time::Duration::seconds(60));

    for (installation_id, generation_id, route_text) in [
        (
            fixture.global_installation,
            fixture.global_generation,
            "schema-global",
        ),
        (
            fixture.repository_installation,
            fixture.repository_generation,
            "schema-repository",
        ),
    ] {
        let scope_secret = UiBrowserHandoffSecret::random();
        let scope_digest = scope_secret.digest().as_bytes();
        let scope_created = store
            .create_ui_browser_handoff(CreateUiBrowserHandoff {
                request_id: RequestId::new(),
                actor_id: actor,
                parent_session_id: parent,
                installation_id: UiInstallationId::from_uuid(installation_id),
                generation_id: UiInstallationGenerationId::from_uuid(generation_id),
                route: UiBrowserRoute::parse(route_text).expect("scope route"),
                secret: scope_secret,
            })
            .await
            .expect("active actor may issue for every owner scope");
        assert_eq!(
            scope_created.organization_id.as_uuid(),
            fixture.organization
        );
        let stored_scope_digest: Vec<u8> =
            sqlx::query_scalar("SELECT handoff_digest FROM ui_browser_handoffs WHERE id = $1")
                .bind(scope_created.handoff_id.as_uuid())
                .fetch_one(&worker)
                .await
                .expect("read scope handoff digest");
        assert_eq!(stored_scope_digest, scope_digest);
    }

    let baseline: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count issued handoffs");

    let expiring_parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, statement_timestamp(),
                 statement_timestamp() + interval '1 second')",
    )
    .bind(expiring_parent)
    .bind(digest(222))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(223))
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("seed lock-barrier parent");
    let mut account_lock = bootstrap.begin().await.expect("begin account lock barrier");
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *account_lock)
        .await
        .expect("read account lock barrier PID");
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(fixture.actor)
        .fetch_one(&mut *account_lock)
        .await
        .expect("hold actor account lock barrier");
    let barrier_store = PgUiBrowserSessionStore::new(worker.clone(), barrier_app);
    let barrier_route = route.clone();
    let barrier_task = tokio::spawn(async move {
        barrier_store
            .create_ui_browser_handoff(CreateUiBrowserHandoff {
                request_id: RequestId::new(),
                actor_id: actor,
                parent_session_id: BrowserSessionId::from_uuid(expiring_parent),
                installation_id: installation,
                generation_id: generation,
                route: barrier_route,
                secret: UiBrowserHandoffSecret::random(),
            })
            .await
    });
    wait_for_named_lock_waiter(&bootstrap, "ui-browser-hephaestus_worker", blocker_pid).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    account_lock
        .commit()
        .await
        .expect("release account lock barrier");
    assert_eq!(
        barrier_task.await.expect("expiry barrier task"),
        Err(UiBrowserHandoffError::PermissionDenied)
    );
    let after_barrier: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count after expiry barrier");
    assert_eq!(after_barrier, baseline);

    let route_denied_request_id = RequestId::new();
    let route_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: route_denied_request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("arbitrary").expect("safe undeclared route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(route_denied, Err(UiBrowserHandoffError::InvalidRoute));
    assert_audit_row(
        &bootstrap,
        route_denied_request_id,
        "handoff_issue",
        "denied",
        "not_attempted",
        "invalid_route",
    )
    .await;
    assert_audit_actor_only(&bootstrap, route_denied_request_id, fixture.actor).await;
    let after_route: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count after route denial");
    assert_eq!(after_route, baseline);

    sqlx::query("REVOKE INSERT ON public.ui_request_audit_events FROM hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("revoke audit insert for denied-operation test");
    let unavailable_audit_request_id = RequestId::new();
    let denied_with_unavailable_audit = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: unavailable_audit_request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("arbitrary").expect("safe undeclared route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(
        denied_with_unavailable_audit,
        Err(UiBrowserHandoffError::InvalidRoute)
    );
    sqlx::query("GRANT INSERT ON public.ui_request_audit_events TO hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("restore audit insert after denied-operation test");
    let unavailable_audit_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_request_audit_events WHERE request_id = $1")
            .bind(unavailable_audit_request_id.as_uuid())
            .fetch_one(&bootstrap)
            .await
            .expect("count unavailable audit rows");
    assert_eq!(unavailable_audit_rows, 0);

    let revoked_parent_id = Uuid::new_v4();
    insert_canonical_session(
        &worker,
        revoked_parent_id,
        fixture.actor,
        Uuid::new_v4(),
        20,
    )
    .await;
    sqlx::query(
        "UPDATE human_browser_sessions
         SET revoked_at = statement_timestamp(), revocation_reason = 'administrative'
         WHERE id = $1",
    )
    .bind(revoked_parent_id)
    .execute(&worker)
    .await
    .expect("revoke parent session");
    let revoked_parent = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: BrowserSessionId::from_uuid(revoked_parent_id),
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(revoked_parent, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("suspend actor account");
    let inactive_actor = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(inactive_actor, Err(UiBrowserHandoffError::PermissionDenied));
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore actor account");

    let future_parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, statement_timestamp() + interval '1 hour',
                 statement_timestamp() + interval '2 hours')",
    )
    .bind(future_parent)
    .bind(digest(224))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(225))
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("seed future-issued parent");
    let future_issued = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: BrowserSessionId::from_uuid(future_parent),
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(future_issued, Err(UiBrowserHandoffError::PermissionDenied));

    let actor_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: UserId::from_uuid(fixture.outsider),
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(actor_denied, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("UPDATE ui_installations SET lifecycle = 'disabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("disable installation for denial case");
    let disabled_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(
        disabled_denied,
        Err(UiBrowserHandoffError::PermissionDenied)
    );
    sqlx::query("UPDATE ui_installations SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("restore installation for generation case");

    let stale_generation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         SELECT $1, installation_id, generation_no + 1, release_id, ui_key, ui_scope
         FROM ui_installation_generations WHERE id = $2",
    )
    .bind(stale_generation)
    .bind(fixture.generation)
    .execute(&worker)
    .await
    .expect("seed newer generation");
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(stale_generation)
        .execute(&worker)
        .await
        .expect("activate newer generation");
    let stale_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(stale_denied, Err(UiBrowserHandoffError::PermissionDenied));

    let expiring_parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, statement_timestamp(),
                 statement_timestamp() + interval '1 second')",
    )
    .bind(expiring_parent)
    .bind(digest(220))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(221))
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("seed short-lived parent");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let expired_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: BrowserSessionId::from_uuid(expiring_parent),
            installation_id: installation,
            generation_id: UiInstallationGenerationId::from_uuid(stale_generation),
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(expired_denied, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke actor target authority");
    let target_revoked = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.global_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.global_generation),
            route: UiBrowserRoute::parse("schema-global").expect("global route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(target_revoked, Err(UiBrowserHandoffError::PermissionDenied));
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner') ON CONFLICT DO NOTHING",
    )
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("restore actor target authority");

    sqlx::query("UPDATE organization_members SET role = 'member' WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("demote actor for source permission case");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         SELECT project_id, $2 FROM ui_installations WHERE id = $1
         ON CONFLICT DO NOTHING",
    )
    .bind(fixture.other_installation)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("grant source project permission");
    let source_permission_handoff = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("source permission route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await
        .expect("project maintainer may use source release");
    let with_source_permission: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(&worker)
            .await
            .expect("count source permission handoff");
    assert_eq!(with_source_permission, baseline + 1);
    assert_eq!(source_permission_handoff.route.as_str(), "schema-ui-two");
    sqlx::query(
        "DELETE FROM project_maintainers
         WHERE project_id = (SELECT project_id FROM ui_installations WHERE id = $1)
           AND user_id = $2",
    )
    .bind(fixture.other_installation)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("revoke source project permission");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke source release permission");
    let source_permission_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("source permission route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(
        source_permission_denied,
        Err(UiBrowserHandoffError::PermissionDenied)
    );
    sqlx::query("UPDATE organization_members SET role = 'owner' WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore actor owner role");

    let source_release: Uuid =
        sqlx::query_scalar("SELECT release_id FROM ui_installation_generations WHERE id = $1")
            .bind(stale_generation)
            .fetch_one(&worker)
            .await
            .expect("read source release");
    sqlx::query(
        "UPDATE releases
         SET state = 'revoked', revoked_at = statement_timestamp()
         WHERE id = $1",
    )
    .bind(source_release)
    .execute(&worker)
    .await
    .expect("revoke source release");
    let source_revoked = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: UiInstallationGenerationId::from_uuid(stale_generation),
            route,
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(source_revoked, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("UPDATE ui_installations SET lifecycle = 'removed', removed_at = statement_timestamp() WHERE id = $1")
        .bind(fixture.other_installation)
        .execute(&worker)
        .await
        .expect("remove alternate installation");
    let removed_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("alternate route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(removed_denied, Err(UiBrowserHandoffError::PermissionDenied));

    let final_count: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count final denied attempts");
    assert_eq!(final_count, baseline + 1);
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_browser_exchange_is_atomic_generation_bound_and_parent_capped() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser exchange: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0097");
    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let store = PgUiBrowserSessionStore::new(worker.clone(), app);
    let actor = UserId::from_uuid(fixture.actor);
    let parent = BrowserSessionId::from_uuid(fixture.parent_session);
    let installation = UiInstallationId::from_uuid(fixture.installation);
    let generation = UiInstallationGenerationId::from_uuid(fixture.generation);
    let route = UiBrowserRoute::parse("schema-ui").expect("published route base");

    // A host resolved to another generation cannot exchange the locked handoff.
    let wrong_host_secret = UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 41));
    issue_handoff(
        &store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        wrong_host_secret,
    )
    .await;
    let wrong_host_request_id = RequestId::new();
    let wrong_host = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: wrong_host_request_id,
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 41)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 42)),
        })
        .await;
    assert_eq!(wrong_host, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 41)),
    )
    .await;
    assert_audit_row(
        &bootstrap,
        wrong_host_request_id,
        "handoff_exchange",
        "denied",
        "not_attempted",
        "expired",
    )
    .await;
    assert_audit_anonymous(&bootstrap, wrong_host_request_id, "handoff_exchange").await;

    let success_secret = UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 43));
    issue_handoff(
        &store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        success_secret,
    )
    .await;
    let success_request_id = RequestId::new();
    let success = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: success_request_id,
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 43)),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 44)),
        })
        .await
        .expect("valid handoff exchanges once");
    let child_row: (
        Vec<u8>,
        time::OffsetDateTime,
        time::OffsetDateTime,
        Uuid,
        Uuid,
    ) = sqlx::query_as(
        "SELECT session_digest, issued_at, expires_at, parent_session_id, generation_id
         FROM ui_browser_sessions WHERE id = $1",
    )
    .bind(success.context.session_id.as_uuid())
    .fetch_one(&worker)
    .await
    .expect("read child safe metadata");
    assert_eq!(
        child_row.0,
        UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 44))
            .digest()
            .as_bytes()
    );
    assert_eq!(child_row.3, fixture.parent_session);
    assert_eq!(child_row.4, fixture.generation);
    assert_eq!(child_row.2 - child_row.1, time::Duration::hours(12));
    assert_audit_row(
        &bootstrap,
        success_request_id,
        "handoff_exchange",
        "allowed",
        "succeeded",
        "none",
    )
    .await;
    assert_audit_context(
        &bootstrap,
        success_request_id,
        "handoff_exchange",
        fixture.actor,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        Some(success.context.session_id.as_uuid()),
    )
    .await;
    let replay_request_id = RequestId::new();
    let replay = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: replay_request_id,
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 43)),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 45)),
        })
        .await;
    assert_eq!(replay, Err(UiBrowserHandoffError::InvalidOrExpired));
    let replay_children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_browser_sessions WHERE handoff_id =
         (SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1)",
    )
    .bind(
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 43))
            .digest()
            .as_bytes()
            .as_slice(),
    )
    .fetch_one(&worker)
    .await
    .expect("count replay children");
    assert_eq!(replay_children, 1);
    assert_audit_row(
        &bootstrap,
        replay_request_id,
        "handoff_exchange",
        "denied",
        "not_attempted",
        "expired",
    )
    .await;
    assert_audit_anonymous(&bootstrap, replay_request_id, "handoff_exchange").await;

    // An audit persistence failure must roll back both the child insertion and
    // one-time handoff consumption. Restoring the grant must allow that same
    // handoff to be retried successfully.
    let rollback_handoff_secret_bytes = test_secret(fixture.actor, 60);
    let rollback_handoff_digest = UiBrowserHandoffSecret::from_bytes(rollback_handoff_secret_bytes)
        .digest()
        .as_bytes()
        .to_vec();
    issue_handoff(
        &store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        UiBrowserHandoffSecret::from_bytes(rollback_handoff_secret_bytes),
    )
    .await;
    let rollback_handoff_id: Uuid =
        sqlx::query_scalar("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1")
            .bind(&rollback_handoff_digest)
            .fetch_one(&worker)
            .await
            .expect("find audit rollback handoff");
    let rollback_children_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("count audit rollback children before failure");
    let rollback_consumed_before: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("read audit rollback handoff before failure");
    assert!(rollback_consumed_before.is_none());

    sqlx::query("REVOKE INSERT ON public.ui_request_audit_events FROM hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("revoke audit insert for exchange rollback");
    let rollback_request_id = RequestId::new();
    let rollback = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: rollback_request_id,
            handoff_secret: UiBrowserHandoffSecret::from_bytes(rollback_handoff_secret_bytes),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 61)),
        })
        .await;
    assert_eq!(rollback, Err(UiBrowserHandoffError::Unavailable));
    sqlx::query("GRANT INSERT ON public.ui_request_audit_events TO hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("restore audit insert after exchange rollback");

    let rollback_children_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("count audit rollback children after failure");
    let rollback_consumed_after: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("read audit rollback handoff after failure");
    assert_eq!(rollback_children_after, rollback_children_before);
    assert!(rollback_consumed_after.is_none());
    let rollback_audit_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_request_audit_events WHERE request_id = $1")
            .bind(rollback_request_id.as_uuid())
            .fetch_one(&bootstrap)
            .await
            .expect("count failed exchange audit rows");
    assert_eq!(rollback_audit_rows, 0);

    let retry_request_id = RequestId::new();
    let retry = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: retry_request_id,
            handoff_secret: UiBrowserHandoffSecret::from_bytes(rollback_handoff_secret_bytes),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 61)),
        })
        .await
        .expect("retry exchange after restoring audit insert");
    let retry_children: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("count retried exchange child");
    let retry_consumed: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("read retried exchange handoff");
    assert_eq!(retry_children, 1);
    assert!(retry_consumed.is_some());
    assert_audit_row(
        &bootstrap,
        retry_request_id,
        "handoff_exchange",
        "allowed",
        "succeeded",
        "none",
    )
    .await;
    assert_audit_context(
        &bootstrap,
        retry_request_id,
        "handoff_exchange",
        fixture.actor,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        Some(retry.context.session_id.as_uuid()),
    )
    .await;
    println!(
        "REAL_UI_BROWSER_EXCHANGE_AUDIT_ROLLBACK=1 child_rollback={} handoff_unconsumed={} retry_children={} retry_consumed={}",
        rollback_children_after == rollback_children_before,
        rollback_consumed_after.is_none(),
        retry_children,
        retry_consumed.is_some(),
    );

    // Two named worker connections contend on one handoff row. An external
    // blocker makes both waits observable before release; after release only
    // one can insert a child and consume the handoff.
    let parallel_worker_a =
        parallel_role_pool(&database_url, "hephaestus_worker", "ui-browser-exchange-a").await;
    let parallel_worker_b =
        parallel_role_pool(&database_url, "hephaestus_worker", "ui-browser-exchange-b").await;
    let parallel_app_a = role_pool(&database_url, "hephaestus_app").await;
    let parallel_app_b = role_pool(&database_url, "hephaestus_app").await;
    let parallel_store_a = PgUiBrowserSessionStore::new(parallel_worker_a, parallel_app_a);
    let parallel_store_b = PgUiBrowserSessionStore::new(parallel_worker_b, parallel_app_b);
    let concurrent_secret = UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 46));
    let concurrent_digest = concurrent_secret.digest().as_bytes().to_vec();
    issue_handoff(
        &store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        concurrent_secret,
    )
    .await;
    let mut exchange_blocker = bootstrap
        .begin()
        .await
        .expect("begin exchange lock barrier");
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *exchange_blocker)
        .await
        .expect("read exchange lock barrier PID");
    sqlx::query("SELECT set_config('application_name', 'ui-browser-exchange-blocker', false)")
        .execute(&mut *exchange_blocker)
        .await
        .expect("name exchange lock barrier");
    println!("REAL_UI_BROWSER_EXCHANGE_BLOCKER=1 blocker_pid={blocker_pid}");
    sqlx::query("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1 FOR UPDATE")
        .bind(&concurrent_digest)
        .fetch_one(&mut *exchange_blocker)
        .await
        .expect("hold exchange handoff lock barrier");
    let first_task = tokio::spawn(async move {
        parallel_store_a
            .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
                request_id: RequestId::new(),
                handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 46)),
                expected_generation_id: generation,
                child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 47)),
            })
            .await
    });
    let second_task = tokio::spawn(async move {
        parallel_store_b
            .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
                request_id: RequestId::new(),
                handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 46)),
                expected_generation_id: generation,
                child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 48)),
            })
            .await
    });
    wait_for_named_exchange_lock_waiter(&bootstrap, "ui-browser-exchange-a", blocker_pid).await;
    wait_for_named_exchange_lock_waiter(&bootstrap, "ui-browser-exchange-b", blocker_pid).await;
    exchange_blocker
        .commit()
        .await
        .expect("release exchange lock barrier");
    let first = first_task.await.expect("first exchange task");
    let second = second_task.await.expect("second exchange task");
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        usize::from(first == Err(UiBrowserHandoffError::InvalidOrExpired))
            + usize::from(second == Err(UiBrowserHandoffError::InvalidOrExpired)),
        1
    );
    let concurrent_children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_browser_sessions WHERE handoff_id =
         (SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1)",
    )
    .bind(
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 46))
            .digest()
            .as_bytes()
            .as_slice(),
    )
    .fetch_one(&worker)
    .await
    .expect("count concurrent children");
    assert_eq!(concurrent_children, 1);

    // A handoff outside its fixed window is rejected after current authority
    // checks and leaves no child.
    let expired_handoff = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route,
          issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 statement_timestamp() - interval '61 seconds',
                 statement_timestamp() - interval '1 second')",
    )
    .bind(expired_handoff)
    .bind(
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 49))
            .digest()
            .as_bytes()
            .as_slice(),
    )
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.organization)
    .bind("schema-ui")
    .execute(&worker)
    .await
    .expect("seed expired handoff");
    let expired = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 49)),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 50)),
        })
        .await;
    assert_eq!(expired, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 49)),
    )
    .await;

    // Current generation and source publication are rechecked at exchange.
    let stale_secret = UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 51));
    issue_handoff(
        &store,
        actor,
        parent,
        installation,
        generation,
        route,
        stale_secret,
    )
    .await;
    let stale_generation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         SELECT $1, installation_id, generation_no + 1, release_id, ui_key, ui_scope
         FROM ui_installation_generations WHERE id = $2",
    )
    .bind(stale_generation)
    .bind(fixture.generation)
    .execute(&worker)
    .await
    .expect("seed current generation replacement");
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(stale_generation)
        .execute(&worker)
        .await
        .expect("activate current generation replacement");
    let stale = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 51)),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 52)),
        })
        .await;
    assert_eq!(stale, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 51)),
    )
    .await;

    // Parent revocation/account state is checked before child creation.
    let revoked_secret = UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 53));
    issue_handoff(
        &store,
        actor,
        parent,
        UiInstallationId::from_uuid(fixture.other_installation),
        UiInstallationGenerationId::from_uuid(fixture.other_generation),
        UiBrowserRoute::parse("schema-ui-two").expect("second published route base"),
        revoked_secret,
    )
    .await;
    sqlx::query(
        "UPDATE human_browser_sessions
         SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
         WHERE id = $1",
    )
    .bind(fixture.parent_session)
    .execute(&worker)
    .await
    .expect("revoke parent session");
    let revoked = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 53)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 54)),
        })
        .await;
    assert_eq!(revoked, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 53)),
    )
    .await;

    let account_parent = Uuid::new_v4();
    insert_canonical_session(&worker, account_parent, fixture.actor, Uuid::new_v4(), 20).await;
    issue_handoff(
        &store,
        actor,
        BrowserSessionId::from_uuid(account_parent),
        UiInstallationId::from_uuid(fixture.other_installation),
        UiInstallationGenerationId::from_uuid(fixture.other_generation),
        UiBrowserRoute::parse("schema-ui-two").expect("second published route base"),
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 57)),
    )
    .await;
    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("suspend account");
    let account_suspended = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 57)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 58)),
        })
        .await;
    assert_eq!(
        account_suspended,
        Err(UiBrowserHandoffError::InvalidOrExpired)
    );
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 57)),
    )
    .await;
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore account");

    let source_parent = Uuid::new_v4();
    insert_canonical_session(&worker, source_parent, fixture.actor, Uuid::new_v4(), 20).await;
    issue_handoff(
        &store,
        actor,
        BrowserSessionId::from_uuid(source_parent),
        UiInstallationId::from_uuid(fixture.other_installation),
        UiInstallationGenerationId::from_uuid(fixture.other_generation),
        UiBrowserRoute::parse("schema-ui-two").expect("second published route base"),
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 55)),
    )
    .await;
    sqlx::query(
        "UPDATE releases SET state = 'revoked', revoked_at = statement_timestamp()
         WHERE id = (SELECT release_id FROM ui_installation_generations WHERE id = $1)",
    )
    .bind(fixture.other_generation)
    .execute(&worker)
    .await
    .expect("revoke source release");
    let source_revoked = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 55)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 56)),
        })
        .await;
    assert_eq!(source_revoked, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 55)),
    )
    .await;
}

async fn assert_exchange_denial_unchanged(pool: &PgPool, secret: UiBrowserHandoffSecret) {
    let digest = secret.digest().as_bytes().to_vec();
    let handoff_id: Uuid =
        sqlx::query_scalar("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1")
            .bind(&digest)
            .fetch_one(pool)
            .await
            .expect("find denied exchange handoff");
    let consumed_at: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(handoff_id)
            .fetch_one(pool)
            .await
            .expect("read denied exchange consumption");
    let children: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(handoff_id)
            .fetch_one(pool)
            .await
            .expect("count denied exchange children");
    assert!(
        consumed_at.is_none(),
        "denied exchange consumed its handoff"
    );
    assert_eq!(children, 0, "denied exchange created a child");
}

async fn issue_handoff(
    store: &PgUiBrowserSessionStore,
    actor: UserId,
    parent: BrowserSessionId,
    installation: UiInstallationId,
    generation: UiInstallationGenerationId,
    route: UiBrowserRoute,
    secret: UiBrowserHandoffSecret,
) {
    store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route,
            secret,
        })
        .await
        .expect("issue exchange fixture handoff");
}

async fn parallel_role_pool(database_url: &str, role: &str, application_name: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect({
            let role = role.to_owned();
            let application_name = application_name.to_owned();
            move |connection, _metadata| {
                let role = role.clone();
                let application_name = application_name.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('role', $1, false)")
                        .bind(role)
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('application_name', $1, false)")
                        .bind(application_name)
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
                        .bind(Uuid::nil().to_string())
                        .execute(&mut *connection)
                        .await
                        .map(|_| ())
                })
            }
        })
        .connect(database_url)
        .await
        .expect("connect parallel restricted PostgreSQL role")
}
