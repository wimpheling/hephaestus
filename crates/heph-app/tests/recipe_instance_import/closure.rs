use hephaestus_app::recipe_instance_import::{InstanceImportError, InstanceImportFailure};
use recipe_application::{ActiveDeploymentEffectRepository, InstallProgress};
use sqlx::PgPool;

use crate::{fixtures, instance_fixture::Harness, races, support};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn admitted_remove_while_actual_insert_waits_prevents_every_prepared_instance_birth_row(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let prepared = h.prepared().await;
    let claim = h.claim().await;
    let (blocker, pid) = races::block_insertion(&pool).await;
    let pending = races::spawn_import(&h);
    races::wait_for_insertion(&pool, pid).await;
    let manager = manager(&pool, &h).await;
    let snapshot = h.repository.inspect(&manager, h.intent.id()).await.unwrap();
    let remove = fixtures::remove(&manager, h.intent.id(), snapshot.version);
    h.repository.admit_remove(&manager, remove).await.unwrap();
    blocker.commit().await.unwrap();
    assert!(matches!(
        pending.await.unwrap(),
        Err(InstanceImportError::Held {
            failure: InstanceImportFailure::Release(_),
            ..
        })
    ));
    let counts:(i64,i64,i64,i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM agent_instances WHERE id=$1),
      (SELECT count(*) FROM agent_instance_revisions WHERE id=$2),
      (SELECT count(*) FROM agent_instance_revision_volume_bindings WHERE instance_id=$1),
      (SELECT count(*) FROM agent_instance_volume_mount_grants WHERE instance_id=$1),
      (SELECT count(*) FROM release_command_inbox WHERE command_key=$3),
      (SELECT count(*) FROM agent_instance_events WHERE instance_id=$1),
      (SELECT count(*) FROM outbox WHERE aggregate_id=$1)+(SELECT count(*) FROM application_events WHERE aggregate_type='agent_instance' AND aggregate_id=$1)")
      .bind(prepared.instance_id().as_uuid()).bind(prepared.revision_id().as_uuid()).bind(prepared.command_key().as_bytes().as_slice()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        counts,
        (0, 0, 0, 0, 0, 0, 0),
        "the actual generic107 transaction rolls back after the serialized Remove wins"
    );
    assert_eq!(
        h.preparation
            .load_active_effect(
                &manager,
                remove.command,
                h.intent.id(),
                &fixtures::key("sqlite")
            )
            .await
            .unwrap(),
        Some(claim)
    );
    let verification: i64 = sqlx::query_scalar("SELECT count(*) FROM recipe_effect_verifications")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        verification, 0,
        "denied insertion is not an invented absence/success outcome"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn import_committed_first_is_visible_to_subsequent_remove_with_exact_protected_completion(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    h.importer()
        .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
    let child = h.prepared().await;
    let manager = manager(&pool, &h).await;
    let snapshot = h.repository.inspect(&manager, h.intent.id()).await.unwrap();
    let remove = fixtures::remove(&manager, h.intent.id(), snapshot.version);
    let admitted = h.repository.admit_remove(&manager, remove).await.unwrap();
    let actual:i64=sqlx::query_scalar("SELECT count(*) FROM agent_instances WHERE id=$1 AND active_revision_id=$2 AND volume_mode='named' AND NOT run_gate_open")
      .bind(child.instance_id().as_uuid()).bind(child.revision_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(actual, 1);
    let progress = &admitted.snapshot.resources[&fixtures::key("sqlite")];
    assert_eq!(progress.install, InstallProgress::Ready);
    assert!(progress.active_attempt.is_none());
    assert!(
        h.importer()
            .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
            .await
            .is_err(),
        "permanent removal never reopens original Install"
    );
}

async fn manager(pool: &PgPool, h: &Harness) -> identity_domain::AuthenticatedIdentity {
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.fixture.consuming_project)
        .bind(h.fixture.outsider.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    support::identity(h.fixture.outsider)
}
