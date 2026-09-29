//! Durable update admission lifecycle fixture transitions.

use super::super::super::rpc::update_admission::{
    RecoveryAction, UpdateAdmissionInstance, UpdateRpcClient, recover_update,
};
use identity_domain::BrowserSessionSid;
use std::time::Duration;
use uuid::Uuid;

pub async fn seed_active_run(pool: &sqlx::PgPool, instance: &UpdateAdmissionInstance) -> Uuid {
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
pub async fn admit_first_update_hook(
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
pub async fn retry_update_hook(
    pool: &sqlx::PgPool,
    client: &UpdateRpcClient,
    instance: &UpdateAdmissionInstance,
    actor: uuid::Uuid,
    sid: BrowserSessionSid,
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
        sid,
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
pub async fn reject_revoked_update_admission(
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

pub async fn wait_for_new_hook_run(
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
