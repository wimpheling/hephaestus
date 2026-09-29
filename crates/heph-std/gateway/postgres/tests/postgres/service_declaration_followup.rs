//! Follow-up phases for the gateway service declaration integration scenario.

use super::support::Fixture;
use forge_domain::{ProjectId, RepositoryId};
use gateway_postgres::{
    ConfigureGatewayRequest, GatewayConfigureError, InstallGatewayManifest,
    PostgresGatewayInstaller, PostgresGatewayManagement,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::ReleaseId;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Exercises service configuration, replay, replacement, and stale-state handling.
// This single scenario preserves the integration test's transaction sequence.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(super) async fn exercise_service_configuration(
    pool: &sqlx::PgPool,
    management: &PostgresGatewayManagement,
    installer: &PostgresGatewayInstaller,
    owner: &AuthenticatedIdentity,
    fixture: &Fixture,
    gateway_id: Uuid,
    installed_revision: Uuid,
    persisted: &(String, Option<i32>, Option<String>, Option<String>, String),
) -> Uuid {
    let outbox_before_desired_change: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM product_event_outbox outbox
           JOIN application_events event ON event.id = outbox.event_id
          WHERE event.aggregate_type = 'gateway' AND event.aggregate_id = $1",
    )
    .bind(gateway_id)
    .fetch_one(pool)
    .await
    .expect("count service product events before desired change");
    let configured = management
        .configure(
            owner,
            ConfigureGatewayRequest {
                gateway_id,
                expected_revision_id: installed_revision,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await
        .expect("clone service revision");
    let pending_state: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT active_revision_id, desired_service_revision_id
           FROM gateways WHERE id = $1",
    )
    .bind(gateway_id)
    .fetch_one(pool)
    .await
    .expect("load pending service replacement state");
    assert_eq!(pending_state.0, None);
    assert_eq!(pending_state.1, Some(configured.revision_id));
    let configured_mode: String =
        sqlx::query_scalar("SELECT service_log_capture_mode FROM gateway_revisions WHERE id = $1")
            .bind(configured.revision_id)
            .fetch_one(pool)
            .await
            .expect("load configured service log mode");
    assert_eq!(configured_mode, "application");
    let outbox_after_desired_change: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM product_event_outbox outbox
           JOIN application_events event ON event.id = outbox.event_id
          WHERE event.aggregate_type = 'gateway' AND event.aggregate_id = $1",
    )
    .bind(gateway_id)
    .fetch_one(pool)
    .await
    .expect("count service product events after desired change");
    assert!(
        outbox_after_desired_change > outbox_before_desired_change,
        "desired-only service changes must invalidate through the product outbox"
    );

    let replay = management
        .configure(
            owner,
            ConfigureGatewayRequest {
                gateway_id,
                expected_revision_id: installed_revision,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await
        .expect("replay the exact service configuration command");
    assert_eq!(replay.revision_id, configured.revision_id);
    let outbox_before_rolled_back_change: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM product_event_outbox outbox
           JOIN application_events event ON event.id = outbox.event_id
          WHERE event.aggregate_type = 'gateway' AND event.aggregate_id = $1",
    )
    .bind(gateway_id)
    .fetch_one(pool)
    .await
    .expect("count service product events before stale change");
    let stale = management
        .configure(
            &AuthenticatedIdentity::new(
                UserId::from_uuid(fixture.owner),
                "test",
                "gateway-service-stale",
                serde_json::json!({}),
                RequestId::new(),
            ),
            ConfigureGatewayRequest {
                gateway_id,
                expected_revision_id: installed_revision,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await;
    assert!(matches!(stale, Err(GatewayConfigureError::Stale)));
    let outbox_after_rolled_back_change: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM product_event_outbox outbox
           JOIN application_events event ON event.id = outbox.event_id
          WHERE event.aggregate_type = 'gateway' AND event.aggregate_id = $1",
    )
    .bind(gateway_id)
    .fetch_one(pool)
    .await
    .expect("count service product events after stale change");
    assert_eq!(
        outbox_after_rolled_back_change, outbox_before_rolled_back_change,
        "a rolled-back stale change must not emit a product event"
    );

    let old_stateless_revision = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, mailbox_slots, normalized_hash, created_by)
         SELECT $1, $2, project_id, repository_id, release_id,
                release_agent_id, release_agent_key, 'http.v1', exposure,
                parameters, secret_slots, mailbox_slots, $3, created_by
           FROM gateway_revisions WHERE id = $4",
    )
    .bind(old_stateless_revision)
    .bind(gateway_id)
    .bind(Sha256::digest(old_stateless_revision.as_bytes()).as_slice())
    .bind(fixture.revision)
    .execute(pool)
    .await
    .expect("create old stateless revision for pending-service race");
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/old-stateless', ARRAY['GET'])",
    )
    .bind(Uuid::new_v4())
    .bind(old_stateless_revision)
    .bind(gateway_id)
    .bind(fixture.project)
    .execute(pool)
    .await
    .expect("create old stateless route for pending-service race");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(old_stateless_revision)
        .execute(pool)
        .await
        .expect("restore old stateless revision as serving target");
    let stale_active = management
        .configure(
            &AuthenticatedIdentity::new(
                UserId::from_uuid(fixture.owner),
                "test",
                "gateway-service-old-active",
                serde_json::json!({}),
                RequestId::new(),
            ),
            ConfigureGatewayRequest {
                gateway_id,
                expected_revision_id: old_stateless_revision,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await;
    assert!(
        matches!(stale_active, Err(GatewayConfigureError::Stale)),
        "a pending service candidate must make an older active revision stale"
    );

    let stateless_install = installer
        .install(
            &AuthenticatedIdentity::new(
                UserId::from_uuid(fixture.owner),
                "test",
                "gateway-service-stateless-reinstall",
                serde_json::json!({}),
                RequestId::new(),
            ),
            InstallGatewayManifest {
                project_id: ProjectId::from_uuid(fixture.project),
                repository_id: RepositoryId::from_uuid(fixture.repository),
                release_id: Some(ReleaseId::from_uuid(fixture.release)),
                manifest: br#"
version = 1

[[gateways]]
name = "service-installed"
agent_name = "gateway-mailbox"
handler_contract = "http.v1"
exposure = "public"

[[gateways.routes]]
path = "/service-stateless"
methods = ["GET"]
"#
                .to_vec(),
            },
        )
        .await
        .expect("stateless reinstall");
    let cleared_state: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT active_revision_id, desired_service_revision_id
           FROM gateways WHERE id = $1",
    )
    .bind(gateway_id)
    .fetch_one(pool)
    .await
    .expect("load stateless reinstall state");
    assert_eq!(
        cleared_state,
        (
            Some(stateless_install.gateways[0].revision_id.as_uuid()),
            None
        )
    );
    let cloned: (String, Option<i32>, Option<String>, Option<String>, String) = sqlx::query_as(
        "SELECT handler_contract, service_loopback_port, service_readiness_path,
                service_health_path, service_log_capture_mode
         FROM gateway_revisions WHERE id = $1",
    )
    .bind(configured.revision_id)
    .fetch_one(pool)
    .await
    .expect("load cloned service declaration");
    assert_eq!(cloned, persisted.clone());

    sqlx::query(
        "UPDATE gateways SET active_revision_id = $2
          WHERE id = $1",
    )
    .bind(gateway_id)
    .bind(configured.revision_id)
    .execute(pool)
    .await
    .expect("promote service candidate for replacement proof");
    let replacement = management
        .configure(
            &AuthenticatedIdentity::new(
                UserId::from_uuid(fixture.owner),
                "test",
                "gateway-service-replacement",
                serde_json::json!({}),
                RequestId::new(),
            ),
            ConfigureGatewayRequest {
                gateway_id,
                expected_revision_id: configured.revision_id,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await
        .expect("configure replacement while service is serving");
    let replacement_state: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT active_revision_id, desired_service_revision_id
           FROM gateways WHERE id = $1",
    )
    .bind(gateway_id)
    .fetch_one(pool)
    .await
    .expect("load serving and pending replacement state");
    assert_eq!(replacement_state.0, Some(configured.revision_id));
    assert_eq!(replacement_state.1, Some(replacement.revision_id));

    configured.revision_id
}
