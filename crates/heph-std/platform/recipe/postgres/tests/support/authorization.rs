use std::collections::BTreeMap;

use identity_domain::RequestId;
use recipe_application::{
    DeploymentError, DeploymentLifecycle, RemovalProgress, ResourceOwnership,
};
use recipe_postgres::PostgresDeploymentRepository;
use sqlx::PgPool;

use crate::{fixtures, seed, support};

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn source_revocation_denies_install_replay_but_allows_inspect_and_cleanup(pool: PgPool) {
    let fixture = seed::seed(&pool).await;
    let repository = PostgresDeploymentRepository::new(fixtures::app_pool(&pool).await);
    let identity = support::identity(fixture.maintainer);
    let intent = fixtures::intent(&fixture, false);
    repository
        .admit_install(&identity, fixtures::install(&identity, intent.clone()))
        .await
        .expect("install");
    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(fixture.project)
        .bind(fixture.maintainer.as_uuid())
        .execute(&pool)
        .await
        .expect("revoke source access");
    let mut retry = identity.clone();
    retry.request_id = RequestId::new();
    assert!(matches!(
        repository
            .admit_install(&retry, fixtures::install(&retry, intent.clone()))
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    let denied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
        WHERE request_id = $1 AND decision = 'deny' AND object_id = $2 AND permission = 'can_use'",
    )
    .bind(retry.request_id.as_uuid())
    .bind(fixture.release_agent)
    .fetch_one(&pool)
    .await
    .expect("durable denied audit");
    assert_eq!(denied, 1);
    assert_eq!(
        repository
            .inspect(&identity, intent.id())
            .await
            .expect("historical inspect")
            .intent,
        intent
    );
    let cleanup = support::identity(fixture.maintainer);
    let remove = fixtures::remove(&cleanup, intent.id(), 0);
    let admitted = repository
        .admit_remove(&cleanup, remove)
        .await
        .expect("cleanup without source access");
    assert_eq!(admitted.snapshot.lifecycle, DeploymentLifecycle::Removing);
    assert_eq!(admitted.snapshot.version, 1);
    let mut resumed_identity = cleanup;
    resumed_identity.request_id = RequestId::new();
    let resumed = repository
        .admit_remove(&resumed_identity, remove)
        .await
        .expect("original CAS input replay");
    assert_eq!(resumed.receipt, admitted.receipt);
    let changed = fixtures::remove(&resumed_identity, intent.id(), 1);
    assert!(matches!(
        repository.admit_remove(&resumed_identity, changed).await,
        Err(DeploymentError::InputConflict)
    ));
    let outsider = support::identity(fixture.outsider);
    assert!(matches!(
        repository.inspect(&outsider, intent.id()).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    let outsider_remove = fixtures::remove(&outsider, intent.id(), 1);
    assert!(matches!(
        repository.admit_remove(&outsider, outsider_remove).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn historical_external_evidence_survives_loss_and_cleanup_never_deletes_external(
    pool: PgPool,
) {
    let fixture = seed::seed(&pool).await;
    let repository = PostgresDeploymentRepository::new(fixtures::app_pool(&pool).await);
    let identity = support::identity(fixture.maintainer);
    let intent = fixtures::intent(&fixture, true);
    let admitted = repository
        .admit_install(&identity, fixtures::install(&identity, intent.clone()))
        .await
        .expect("external admission");
    let data = fixtures::key("data");
    assert_eq!(
        admitted.snapshot.intent.resources()[&data].ownership(),
        ResourceOwnership::External
    );
    assert_eq!(
        admitted.snapshot.resources[&data].removal,
        RemovalProgress::Retained
    );
    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(fixture.release)
        .execute(&pool)
        .await
        .expect("withdraw release");
    sqlx::query("DELETE FROM agent_instance_state_volumes WHERE id = $1")
        .bind(fixture.volume)
        .execute(&pool)
        .await
        .expect("lose external resource");
    let historical = repository
        .inspect(&identity, intent.id())
        .await
        .expect("immutable historical evidence");
    assert_eq!(historical.intent, intent);
    let cleanup = support::identity(fixture.maintainer);
    let removed = repository
        .admit_remove(&cleanup, fixtures::remove(&cleanup, intent.id(), 0))
        .await
        .expect("admit cleanup");
    assert_eq!(
        removed.snapshot.resources[&data].removal,
        RemovalProgress::Retained
    );
    assert_eq!(removed.snapshot.intent, intent);
    assert_eq!(
        admitted.receipt.event.event_id,
        sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT event_id FROM recipe_deployment_commands WHERE id = $1"
        )
        .bind(admitted.receipt.command.id().as_uuid())
        .fetch_one(&pool)
        .await
        .expect("original receipt")
    );
    eprintln!(
        "REAL_RECIPE_LEDGER_CLEANUP=1 withdrawn_source_inspect=1 external_loss_inspect=1 external_retained=1 no_source_authority_for_cleanup=1"
    );
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn trusted_catalog_rejects_forged_slots_and_required_secret_omission(pool: PgPool) {
    let fixture = seed::seed(&pool).await;
    let repository = PostgresDeploymentRepository::new(fixtures::app_pool(&pool).await);
    let identity = support::identity(fixture.maintainer);
    let original = fixtures::intent(&fixture, false);
    let mut forged_catalog = fixtures::catalog(&fixture);
    let slot = &forged_catalog.volume_slots[0];
    forged_catalog.volume_slots[0] = volume_domain::VolumeSlotDeclaration::new(
        slot.slot().clone(),
        slot.guest_path().clone(),
        slot.access_mode(),
        slot.required(),
        16_777_216,
    )
    .expect("forged caller ceiling");
    let forged = fixtures::build(
        &fixture,
        &fixtures::source(&fixture, false),
        original.id(),
        "sqlite",
        &BTreeMap::new(),
        &[forged_catalog],
        false,
    );
    assert!(matches!(
        repository
            .admit_install(&identity, fixtures::install(&identity, forged))
            .await,
        Err(DeploymentError::IntentMismatch)
    ));
    let secret_fixture = fixtures::required_secret_release(&pool, &fixture).await;
    let omitted = fixtures::intent(&secret_fixture, false);
    assert!(matches!(
        repository
            .admit_install(&identity, fixtures::install(&identity, omitted))
            .await,
        Err(DeploymentError::Recipe(_))
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM recipe_deployments")
        .fetch_one(&pool)
        .await
        .expect("rollback forged admission");
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn unselected_external_reference_audits_read_without_attach_or_grant(pool: PgPool) {
    let fixture = seed::seed(&pool).await;
    let repository = PostgresDeploymentRepository::new(fixtures::app_pool(&pool).await);
    let identity = support::identity(fixture.maintainer);
    let full_source = fixtures::source(&fixture, true);
    let source = full_source
        .split("[[resources]]\nkind = \"instance\"")
        .next()
        .expect("volume source");
    let intent = fixtures::build(
        &fixture,
        source,
        recipe_application::DeploymentId::from_uuid(uuid::Uuid::new_v4()).expect("id"),
        "reference",
        &BTreeMap::new(),
        &[],
        true,
    );
    repository
        .admit_install(&identity, fixtures::install(&identity, intent))
        .await
        .expect("external reference");
    let permissions: Vec<String> = sqlx::query_scalar("SELECT permission FROM authorization_audit_events
        WHERE actor_id = $1 AND request_id = $2 AND object_type = 'state_volume' AND object_id = $3 ORDER BY permission")
        .bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(fixture.volume)
        .fetch_all(&pool).await.expect("exact audited operations");
    assert_eq!(permissions, vec!["can_read"]);
}
