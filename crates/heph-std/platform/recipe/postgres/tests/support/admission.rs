use std::collections::BTreeMap;

use identity_domain::RequestId;
use recipe_application::{
    AdmissionDisposition, DeploymentError, DeploymentId, DeploymentLifecycle,
};
use recipe_postgres::PostgresDeploymentRepository;
use release_domain::{ParameterName, ParameterValue};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{fixtures, seed, support};

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn admission_replay_keeps_original_receipt_and_append_only_provenance(pool: PgPool) {
    let fixture = seed::seed(&pool).await;
    let app = fixtures::app_pool(&pool).await;
    let repository = PostgresDeploymentRepository::new(app);
    let identity = support::identity(fixture.maintainer);
    let intent = fixtures::intent(&fixture, false);
    let created = repository
        .admit_install(&identity, fixtures::install(&identity, intent.clone()))
        .await
        .expect("admit");
    assert_eq!(created.disposition, AdmissionDisposition::Created);
    assert_eq!(created.snapshot.lifecycle, DeploymentLifecycle::Installing);
    assert_eq!(created.snapshot.version, 0);
    let mut retry = identity.clone();
    retry.request_id = RequestId::new();
    let resumed = repository
        .admit_install(&retry, fixtures::install(&retry, intent.clone()))
        .await
        .expect("resume");
    assert_eq!(resumed.disposition, AdmissionDisposition::Resume);
    assert_eq!(resumed.receipt, created.receipt);
    assert_eq!(resumed.snapshot.intent, intent);
    let attempts: Vec<Uuid> = sqlx::query_scalar("SELECT request_id FROM recipe_deployment_admission_attempts WHERE deployment_id = $1 ORDER BY created_at")
        .bind(intent.id().as_uuid()).fetch_all(&pool).await.expect("provenance");
    assert_eq!(
        attempts,
        vec![identity.request_id.as_uuid(), retry.request_id.as_uuid()]
    );
    let event: (Uuid, i64, i64) = sqlx::query_as("SELECT event.actor_id, event.cursor, event.aggregate_version
        FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id = event.id WHERE event.id = $1")
        .bind(created.receipt.event.event_id).fetch_one(&pool).await.expect("committed outbox");
    assert_eq!(
        event,
        (
            fixture.maintainer.as_uuid(),
            created.receipt.event.cursor,
            created.receipt.event.aggregate_version
        )
    );
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_capability_bindings binding JOIN agent_instance_revisions revision ON revision.id = binding.instance_revision_id WHERE revision.instance_id IN
        (SELECT resource_id FROM recipe_deployment_resources WHERE deployment_id = $1)")
        .bind(intent.id().as_uuid()).fetch_one(&pool).await.expect("no fabricated consumer grants");
    assert_eq!(grants, 0);
    eprintln!(
        "REAL_RECIPE_LEDGER_ADMISSION=1 immutable_intent=1 stable_receipt=1 append_only_requests=1 committed_outbox=1 no_provider_effects=1"
    );
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn input_recipe_version_and_project_key_conflicts_are_rejected(pool: PgPool) {
    let fixture = seed::seed(&pool).await;
    let repository = PostgresDeploymentRepository::new(fixtures::app_pool(&pool).await);
    let identity = support::identity(fixture.maintainer);
    let intent = fixtures::intent(&fixture, false);
    repository
        .admit_install(&identity, fixtures::install(&identity, intent.clone()))
        .await
        .expect("initial admission");
    let inputs = BTreeMap::from([(
        ParameterName::parse("capacity").expect("name"),
        ParameterValue::Integer(33_554_432),
    )]);
    let changed = fixtures::build(
        &fixture,
        &fixtures::source(&fixture, false),
        intent.id(),
        "sqlite",
        &inputs,
        &[fixtures::catalog(&fixture)],
        false,
    );
    assert!(matches!(
        repository
            .admit_install(&identity, fixtures::install(&identity, changed))
            .await,
        Err(DeploymentError::InputConflict)
    ));
    let new_identity = support::identity(fixture.maintainer);
    let reused = fixtures::intent(&fixture, false);
    assert!(matches!(
        repository
            .admit_install(&new_identity, fixtures::install(&new_identity, reused))
            .await,
        Err(DeploymentError::InputConflict)
    ));
    let changed_source =
        fixtures::source(&fixture, false).replace("default = 16777216", "default = 33554432");
    let new_recipe = fixtures::build(
        &fixture,
        &changed_source,
        DeploymentId::from_uuid(Uuid::new_v4()).expect("id"),
        "different-key",
        &BTreeMap::new(),
        &[fixtures::catalog(&fixture)],
        false,
    );
    assert!(matches!(
        repository
            .admit_install(&new_identity, fixtures::install(&new_identity, new_recipe))
            .await,
        Err(DeploymentError::InputConflict)
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM recipe_deployments")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn concurrent_same_command_admits_one_deployment_and_one_event(pool: PgPool) {
    let fixture = seed::seed(&pool).await;
    let repository = PostgresDeploymentRepository::new(fixtures::app_pool(&pool).await);
    let identity = support::identity(fixture.maintainer);
    let intent = fixtures::intent(&fixture, false);
    let mut retry = identity.clone();
    retry.request_id = RequestId::new();
    let (first, second) = tokio::join!(
        repository.admit_install(&identity, fixtures::install(&identity, intent.clone())),
        repository.admit_install(&retry, fixtures::install(&retry, intent))
    );
    let first = first.expect("first concurrent admission");
    let second = second.expect("second concurrent admission");
    assert_eq!(first.receipt, second.receipt);
    assert_ne!(first.disposition, second.disposition);
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM recipe_deployments),
        (SELECT count(*) FROM recipe_deployment_commands), (SELECT count(*) FROM recipe_deployment_admission_attempts)")
        .fetch_one(&pool).await.expect("durable counts");
    assert_eq!(counts, (1, 1, 2));
}
