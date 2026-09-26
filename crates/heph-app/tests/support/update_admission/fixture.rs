//! Public update admission exercises and command preparation.

use super::super::rpc::update_admission::{
    UpdateAdmissionInstance, UpdateAdmissionResult, UpdateRpcClient, app_instance_client,
    create_draining_update,
};
#[cfg(feature = "test-fixtures")]
use hephaestus_app::test_hooks::{
    CreateUpdateAdmissionBarrier, install_create_update_admission_barrier,
};
use identity_domain::BrowserSessionSid;
#[cfg(feature = "test-fixtures")]
use std::sync::Arc;
#[cfg(feature = "test-fixtures")]
use std::time::Duration;
#[cfg(feature = "test-fixtures")]
use uuid::Uuid;

#[path = "../update_admission/lifecycle.rs"]
mod lifecycle;
#[path = "../update_admission/recovery.rs"]
mod recovery;

use self::lifecycle::{
    admit_first_update_hook, reject_revoked_update_admission, retry_update_hook, seed_active_run,
};
use self::recovery::recover_update_with_new_owner;

/// Exercises the public app command while a persisted normal run still owns
/// the instance. This is intentionally opt-in so the ordinary golden path
/// remains independent of the synthetic active-run fixture.
pub async fn exercise(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    instance: &UpdateAdmissionInstance,
    actor: uuid::Uuid,
    sid: BrowserSessionSid,
) -> UpdateAdmissionResult {
    let (client, active_run, update_id) =
        prepare_update_admission(pool, running, instance, actor, sid).await;
    let hook_run_id = admit_first_update_hook(pool, active_run, update_id).await;
    let retry_hook_run_id =
        retry_update_hook(pool, &client, instance, actor, sid, update_id, hook_run_id).await;
    reject_revoked_update_admission(pool, instance, actor, update_id, retry_hook_run_id).await;
    recover_update_with_new_owner(
        pool,
        &client,
        instance,
        update_id,
        hook_run_id,
        retry_hook_run_id,
    )
    .await
}

/// Reproduces the idle reconciler race at the public `CreateUpdate` command.
///
/// The command is paused after its update transaction commits. The active
/// normal run is then cleaned up and the daemon's durable reconciler admits the
/// hook first. Releasing the command forces its immediate attempt through the
/// production lifecycle fallback, which must return the exact persisted hook
/// identity without creating a second run or command.
#[cfg(feature = "test-fixtures")]
pub async fn exercise_reconciler_wins_race(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    instance: &UpdateAdmissionInstance,
    actor: Uuid,
    sid: BrowserSessionSid,
) -> UpdateAdmissionResult {
    let barrier = Arc::new(CreateUpdateAdmissionBarrier::new());
    let _guard = install_create_update_admission_barrier(Arc::clone(&barrier));
    let active_run = seed_active_run(pool, instance).await;
    let client = app_instance_client(running).await;
    let instance_copy = *instance;
    let create_task =
        tokio::spawn(
            async move { create_draining_update(&client, &instance_copy, actor, sid).await },
        );
    let update_id = barrier.wait_committed().await;
    sqlx::query(
        "UPDATE runs SET state = 'cleaned_up', outcome = 'succeeded', updated_at = now()
         WHERE id = $1",
    )
    .bind(active_run)
    .execute(pool)
    .await
    .expect("clean active run for reconciler race");
    tokio::time::timeout(Duration::from_secs(10), barrier.wait_admitted(update_id))
        .await
        .expect("durable reconciler admission timeout");
    let winning_hook_run_id: Uuid = sqlx::query_scalar(
        "SELECT hook_run_id FROM agent_updates WHERE id = $1 AND hook_run_id IS NOT NULL",
    )
    .bind(update_id)
    .fetch_one(pool)
    .await
    .expect("durable reconciler hook identity");
    barrier.release();
    let (response_update_id, response_hook_run_id) =
        create_task.await.expect("CreateUpdate race task");
    assert_eq!(response_update_id, update_id);
    assert_eq!(response_hook_run_id, Some(winning_hook_run_id));
    let run_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs
          WHERE id = $1 AND run_kind = 'update'",
    )
    .bind(winning_hook_run_id)
    .fetch_one(pool)
    .await
    .expect("single reconciler hook run");
    assert_eq!(run_count, 1);
    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM release_command_inbox
          WHERE aggregate_id = $1 AND operation = 'begin_update_hook'",
    )
    .bind(update_id)
    .fetch_one(pool)
    .await
    .expect("single update admission command");
    assert_eq!(command_count, 1);
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
          WHERE aggregate_id = $1
            AND event_type = 'agent_update.hook_started.v1'",
    )
    .bind(update_id)
    .fetch_one(pool)
    .await
    .expect("single update hook outbox event");
    assert_eq!(outbox_count, 1);
    UpdateAdmissionResult {
        update_id,
        initial_hook_run_id: winning_hook_run_id,
        retried_hook_run_id: winning_hook_run_id,
        owner_recovery_hook_run_id: winning_hook_run_id,
    }
}

/// Seeds the persisted active run and sends the real `CreateUpdate` RPC.
async fn prepare_update_admission(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    instance: &UpdateAdmissionInstance,
    actor: uuid::Uuid,
    sid: BrowserSessionSid,
) -> (UpdateRpcClient, uuid::Uuid, uuid::Uuid) {
    let active_run = seed_active_run(pool, instance).await;

    let client = app_instance_client(running).await;
    let (update_id, hook_run_id) = create_draining_update(&client, instance, actor, sid).await;
    let before_cleanup: (String, Option<uuid::Uuid>) =
        sqlx::query_as("SELECT state, hook_run_id FROM agent_updates WHERE id = $1")
            .bind(update_id)
            .fetch_one(pool)
            .await
            .expect("accepted draining update state");
    assert_eq!(before_cleanup, (String::from("draining"), hook_run_id));

    (client, active_run, update_id)
}
