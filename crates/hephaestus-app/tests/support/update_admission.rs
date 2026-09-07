//! Durable SQL fixtures and assertions for update admission regressions.
//!
//! Generated request construction remains in `support::rpc::update_admission`.

use super::rpc::update_admission::{
    RecoveryAction, UpdateAdmissionInstance, UpdateAdmissionResult, UpdateRpcClient,
    app_instance_client, create_draining_update, recover_update,
};
#[cfg(feature = "test-fixtures")]
use hephaestus_app::test_hooks::{
    CreateUpdateAdmissionBarrier, install_create_update_admission_barrier,
};
#[cfg(feature = "test-fixtures")]
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Exercises the public app command while a persisted normal run still owns
/// the instance. This is intentionally opt-in so the ordinary golden path
/// remains independent of the synthetic active-run fixture.
pub async fn exercise(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    instance: &UpdateAdmissionInstance,
    actor: uuid::Uuid,
) -> UpdateAdmissionResult {
    let (client, active_run, update_id) =
        prepare_update_admission(pool, running, instance, actor).await;
    let hook_run_id = admit_first_update_hook(pool, active_run, update_id).await;
    let retry_hook_run_id =
        retry_update_hook(pool, &client, instance, actor, update_id, hook_run_id).await;
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
) -> UpdateAdmissionResult {
    let barrier = Arc::new(CreateUpdateAdmissionBarrier::new());
    let _guard = install_create_update_admission_barrier(Arc::clone(&barrier));
    let active_run = seed_active_run(pool, instance).await;
    let client = app_instance_client(running).await;
    let instance_copy = *instance;
    let create_task =
        tokio::spawn(async move { create_draining_update(&client, &instance_copy, actor).await });
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
) -> (UpdateRpcClient, uuid::Uuid, uuid::Uuid) {
    let active_run = seed_active_run(pool, instance).await;

    let client = app_instance_client(running).await;
    let (update_id, hook_run_id) = create_draining_update(&client, instance, actor).await;
    let before_cleanup: (String, Option<uuid::Uuid>) =
        sqlx::query_as("SELECT state, hook_run_id FROM agent_updates WHERE id = $1")
            .bind(update_id)
            .fetch_one(pool)
            .await
            .expect("accepted draining update state");
    assert_eq!(before_cleanup, (String::from("draining"), hook_run_id));

    (client, active_run, update_id)
}

async fn seed_active_run(pool: &sqlx::PgPool, instance: &UpdateAdmissionInstance) -> Uuid {
    let active_run = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs
           (id, instance_id, instance_revision_id, release_id, release_agent_id,
            attachment_id, run_kind, command_id, state, requires_state,
            created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'normal', $7, 'running', true, now(), now())",
    )
    .bind(active_run)
    .bind(instance.instance_id)
    .bind(instance.revision_id)
    .bind(instance.release_id)
    .bind(instance.release_agent_id)
    .bind(instance.attachment_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("persist active normal update-drain fixture");
    sqlx::query("UPDATE agent_instances SET run_gate_open = true WHERE id = $1")
        .bind(instance.instance_id)
        .execute(pool)
        .await
        .expect("open gate for app update command");
    active_run
}

/// Finishes the active normal run and waits for exactly one durable hook run.
async fn admit_first_update_hook(
    pool: &sqlx::PgPool,
    active_run: uuid::Uuid,
    update_id: uuid::Uuid,
) -> uuid::Uuid {
    sqlx::query(
        "UPDATE runs SET state = 'cleaned_up', outcome = 'succeeded', updated_at = now()
         WHERE id = $1",
    )
    .bind(active_run)
    .execute(pool)
    .await
    .expect("clean active normal update-drain fixture");
    let hook_run_id = wait_for_new_hook_run(pool, update_id, None).await;
    let run_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE id = $1 AND run_kind = 'update'")
            .bind(hook_run_id)
            .fetch_one(pool)
            .await
            .expect("one resumed update run");
    assert_eq!(run_count, 1);

    hook_run_id
}

/// Retries a failed hook and verifies a fresh deterministic hook generation.
async fn retry_update_hook(
    pool: &sqlx::PgPool,
    client: &UpdateRpcClient,
    instance: &UpdateAdmissionInstance,
    actor: uuid::Uuid,
    update_id: uuid::Uuid,
    hook_run_id: uuid::Uuid,
) -> uuid::Uuid {
    sqlx::query(
        "UPDATE runs SET state = 'cleaned_up', outcome = 'failed', exit_signal = 9,
                updated_at = now()
         WHERE id = $1",
    )
    .bind(hook_run_id)
    .execute(pool)
    .await
    .expect("finish first update hook fixture");
    sqlx::query(
        "UPDATE agent_updates SET state = 'compatibility_unknown',
                hook_exit_signal = 9, updated_at = now()
         WHERE id = $1",
    )
    .bind(update_id)
    .execute(pool)
    .await
    .expect("persist uncertain update fixture");
    sqlx::query(
        "UPDATE agent_instances SET state = 'paused_unknown_state', run_gate_open = false
         WHERE id = $1",
    )
    .bind(instance.instance_id)
    .execute(pool)
    .await
    .expect("pause instance at update recovery boundary");
    recover_update(
        client,
        actor,
        update_id,
        RecoveryAction::Retry,
        "app-update-admission-retry",
    )
    .await;
    let retry_hook_run_id = wait_for_new_hook_run(pool, update_id, Some(hook_run_id)).await;
    assert_ne!(hook_run_id, retry_hook_run_id);

    retry_hook_run_id
}

/// Revokes the original actor and verifies periodic admission fails closed.
async fn reject_revoked_update_admission(
    pool: &sqlx::PgPool,
    instance: &UpdateAdmissionInstance,
    actor: uuid::Uuid,
    update_id: uuid::Uuid,
    retry_hook_run_id: uuid::Uuid,
) {
    sqlx::query(
        "UPDATE runs SET state = 'cleaned_up', outcome = 'failed', exit_signal = 9,
                updated_at = now()
         WHERE id = $1",
    )
    .bind(retry_hook_run_id)
    .execute(pool)
    .await
    .expect("finish retried update hook fixture");
    sqlx::query(
        "UPDATE agent_updates SET state = 'compatibility_unknown',
                hook_exit_signal = 9, updated_at = now()
         WHERE id = $1",
    )
    .bind(update_id)
    .execute(pool)
    .await
    .expect("persist retried uncertain update fixture");
    sqlx::query(
        "UPDATE agent_instances SET state = 'paused_unknown_state', run_gate_open = false
         WHERE id = $1",
    )
    .bind(instance.instance_id)
    .execute(pool)
    .await
    .expect("pause retried instance at recovery boundary");
    sqlx::query(
        "UPDATE agent_updates SET state = 'draining', hook_run_id = NULL,
                updated_at = now()
         WHERE id = $1",
    )
    .bind(update_id)
    .execute(pool)
    .await
    .expect("schedule revoked-actor admission fixture");
    sqlx::query(
        "UPDATE agent_instances SET state = 'update_draining', run_gate_open = false
         WHERE id = $1",
    )
    .bind(instance.instance_id)
    .execute(pool)
    .await
    .expect("fence revoked-actor admission fixture");
    sqlx::query("DELETE FROM project_maintainers WHERE user_id = $1")
        .bind(actor)
        .execute(pool)
        .await
        .expect("revoke update authorization");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let revoked_admission: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT hook_run_id FROM agent_updates WHERE id = $1")
            .bind(update_id)
            .fetch_one(pool)
            .await
            .expect("inspect revoked-actor admission");
    assert!(
        revoked_admission.is_none(),
        "revoked actor cannot admit a hook run"
    );
    sqlx::query(
        "UPDATE agent_updates SET state = 'compatibility_unknown',
                hook_exit_signal = 9, updated_at = now()
         WHERE id = $1",
    )
    .bind(update_id)
    .execute(pool)
    .await
    .expect("restore uncertain state for authorized recovery");
    sqlx::query(
        "UPDATE agent_instances SET state = 'paused_unknown_state', run_gate_open = false
         WHERE id = $1",
    )
    .bind(instance.instance_id)
    .execute(pool)
    .await
    .expect("restore paused state for authorized recovery");
}

async fn wait_for_new_hook_run(
    pool: &sqlx::PgPool,
    update_id: uuid::Uuid,
    previous: Option<uuid::Uuid>,
) -> uuid::Uuid {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let current: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT hook_run_id FROM agent_updates WHERE id = $1 AND hook_run_id IS NOT NULL",
        )
        .bind(update_id)
        .fetch_optional(pool)
        .await
        .expect("poll durable update hook identity");
        if let Some(id) = current {
            if previous != Some(id) {
                return id;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "update hook admission timed out"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Authorizes a new owner and verifies explicit recovery gets a fresh hook ID.
async fn recover_update_with_new_owner(
    pool: &sqlx::PgPool,
    client: &UpdateRpcClient,
    instance: &UpdateAdmissionInstance,
    update_id: uuid::Uuid,
    hook_run_id: uuid::Uuid,
    retry_hook_run_id: uuid::Uuid,
) -> UpdateAdmissionResult {
    // A different currently authorized owner can explicitly recover the
    // fenced update. The recovery actor is persisted with the update so
    // subsequent durable admission reauthorizes that owner rather than the
    // revoked creator.
    let recovery_actor = uuid::Uuid::new_v4();
    let project_id: uuid::Uuid =
        sqlx::query_scalar("SELECT project_id FROM agent_instances WHERE id = $1")
            .bind(instance.instance_id)
            .fetch_one(pool)
            .await
            .expect("load update project for recovery owner");
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Recovery Owner')")
        .bind(recovery_actor)
        .execute(pool)
        .await
        .expect("seed recovery owner");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project_id)
        .bind(recovery_actor)
        .execute(pool)
        .await
        .expect("authorize recovery owner");
    recover_update(
        client,
        recovery_actor,
        update_id,
        RecoveryAction::Retry,
        "app-update-admission-owner-retry",
    )
    .await;
    let owner_retry_hook_run_id =
        wait_for_new_hook_run(pool, update_id, Some(retry_hook_run_id)).await;
    assert_ne!(owner_retry_hook_run_id, hook_run_id);
    assert_ne!(owner_retry_hook_run_id, retry_hook_run_id);
    let persisted_actor: uuid::Uuid =
        sqlx::query_scalar("SELECT actor_id FROM agent_updates WHERE id = $1")
            .bind(update_id)
            .fetch_one(pool)
            .await
            .expect("inspect persisted recovery owner");
    assert_eq!(persisted_actor, recovery_actor);
    UpdateAdmissionResult {
        update_id,
        initial_hook_run_id: hook_run_id,
        retried_hook_run_id: retry_hook_run_id,
        owner_recovery_hook_run_id: owner_retry_hook_run_id,
    }
}
