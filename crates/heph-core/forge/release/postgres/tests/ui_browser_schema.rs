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

#[path = "ui_browser_schema/authentication.rs"]
mod authentication;
#[path = "ui_browser_schema/authentication_authority.rs"]
mod authentication_authority;
#[path = "ui_browser_schema/authentication_denials.rs"]
mod authentication_denials;
#[path = "ui_browser_schema/authentication_gateway.rs"]
mod authentication_gateway;
#[path = "ui_browser_schema/authentication_valid.rs"]
mod authentication_valid;
#[path = "ui_browser_schema/binding_assertions.rs"]
mod binding_assertions;
#[path = "ui_browser_schema/child_inserts.rs"]
mod child_inserts;
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
#[path = "ui_browser_schema/handoff_inserts.rs"]
mod handoff_inserts;
#[path = "ui_browser_schema/lifecycle_assertions.rs"]
mod lifecycle_assertions;
#[path = "ui_browser_schema/matrix.rs"]
mod matrix;
#[path = "ui_browser_schema/repository.rs"]
mod repository;
#[path = "ui_browser_schema/role_audit.rs"]
mod role_audit;
#[path = "ui_browser_schema/schema_test_support.rs"]
mod schema_test_support;

use authentication::*;
use authentication_authority::*;
use authentication_denials::*;
use authentication_gateway::*;
use authentication_valid::*;
use binding_assertions::*;
use child_inserts::*;
use fixture::insert_canonical_session;
use fixture::{Fixture, digest, scoped_secret, test_secret};
use fixture_seed::seed_fixture_reusing_installation_helpers;
use handoff_inserts::*;
use lifecycle_assertions::*;
use role_audit::*;
use schema_test_support::*;

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
