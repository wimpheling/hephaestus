use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use identity_domain::RequestId;
use release_service::{InstanceExecutionError, InstanceExecutionService};
use sqlx::{PgPool, Postgres, Transaction};

use super::{behavior::code, fixture::Harness};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn held_instance_denies_late_lock_without_deadlock_or_mutation(pool: PgPool) {
    let h = Harness::new(&pool).await;
    let before = h.rows().await;
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(h.command.instance_id().as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
    assert_contended(&h).await;
    assert_eq!(h.rows().await, before);
    blocker.rollback().await.unwrap();
    h.service()
        .activate_instance(&h.actor, h.command)
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires private114, actual mkfs.ext4 and readonly e2fsck"]
async fn held_global_volume_or_grant_is_transient_and_preserves_physical_bytes(pool: PgPool) {
    let h = Harness::bound(&pool).await;
    let before = h.rows().await;
    let physical = h.physical.as_ref().unwrap();
    let mut volume = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE")
        .bind(physical.volume)
        .execute(&mut *volume)
        .await
        .unwrap();
    assert_contended(&h).await;
    volume.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
    physical.assert_unchanged();
    let mut grant = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instance_volume_mount_grants WHERE instance_revision_id=$1 ORDER BY id FOR UPDATE").bind(h.command.expected_revision_id().as_uuid()).execute(&mut *grant).await.unwrap();
    assert_contended(&h).await;
    grant.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
    physical.assert_unchanged();
    h.service()
        .activate_instance(&h.actor, h.command)
        .await
        .unwrap();
    physical.assert_unchanged();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn concurrent_same_command_has_one_birth_and_original_replay_receipt(pool: PgPool) {
    let h = Harness::new(&pool).await;
    let mut retry = h.actor.clone();
    retry.request_id = RequestId::new();
    let first = h.service();
    let second = h.service();
    let (a, b) = tokio::join!(
        first.activate_instance(&h.actor, h.command),
        second.activate_instance(&retry, h.command)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    let count:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM agent_instance_execution_activations WHERE instance_id=$1),(SELECT count(*) FROM agent_instance_events WHERE instance_id=$1 AND event_type='instance.activated'),(SELECT count(*) FROM outbox WHERE aggregate_id=$1 AND event_type='agent_instance.activated.v1'),(SELECT count(*) FROM runs WHERE instance_id=$1)")
        .bind(h.command.instance_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(count, (1, 1, 1, 0));
    let before = h.rows().await;
    assert_eq!(
        first
            .activate_instance(&retry, h.command)
            .await
            .unwrap()
            .activation_id(),
        h.command.activation_id()
    );
    assert_eq!(h.rows().await, before);
}

async fn assert_contended(h: &Harness) {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        h.service().activate_instance(&h.actor, h.command),
    )
    .await
    .expect("late NOWAIT must not wait or deadlock");
    assert!(matches!(
        result,
        Err(InstanceExecutionError::OutcomeUncertain)
    ));
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error = write(&mut tx, h).await.unwrap_err();
    assert_eq!(
        code(&error).as_deref(),
        Some("55P03"),
        "actual qualified contention, never absence: {error:?}"
    );
    tx.rollback().await.unwrap();
}
async fn write(tx: &mut Transaction<'_, Postgres>, h: &Harness) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT * FROM write_qualified_instance_activation($1,$2,$3,$4,$5,$6,$7)")
        .bind(h.command.activation_id().as_uuid())
        .bind(h.command.command_key().as_bytes().as_slice())
        .bind(h.command.instance_id().as_uuid())
        .bind(h.command.expected_revision_id().as_uuid())
        .bind(1_i64)
        .bind(&h.config_wire)
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
