//! Durable update admission and launch-contract queries.

use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use super::model::VmLaunchContract;

/// Loads the immutable release, revision, attachment, and update-hook contract.
pub async fn load_vm_launch_contract(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Option<VmLaunchContract>, sqlx::Error> {
    sqlx::query_as(
        "SELECT release_agent.runtime_contract,
                revision.effective_runtime_policy,
                release_agent.requires_state,
                release_agent.update_hook,
                release.state AS release_state,
                revision.runnable AS revision_runnable,
                COALESCE((
                    (run.run_kind = 'update' AND instance.state = 'updating')
                    OR (
                        run.run_kind = 'normal'
                        AND instance.state IN ('active', 'update_rejected')
                        AND instance.active_revision_id = revision.id
                        AND attachment.enabled
                        AND attachment.removed_at IS NULL
                    )
                ), false) AS attachment_runnable,
                agent_update.id AS agent_update_id
         FROM runs AS run
         JOIN agent_instances AS instance ON instance.id = run.instance_id
         JOIN agent_instance_revisions AS revision
           ON revision.id = run.instance_revision_id
          AND revision.instance_id = run.instance_id
         JOIN release_agents AS release_agent
           ON release_agent.id = run.release_agent_id
          AND release_agent.release_id = run.release_id
          AND revision.release_agent_id = release_agent.id
         JOIN releases AS release ON release.id = run.release_id
         LEFT JOIN agent_attachments AS attachment
           ON attachment.id = run.attachment_id
          AND attachment.instance_id = run.instance_id
         LEFT JOIN agent_updates AS agent_update
           ON agent_update.hook_run_id = run.id
         WHERE run.id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
}

/// Lists update hook runs whose completion needs reconciliation after restart.
pub async fn recoverable_update_hook_run_ids(pool: &PgPool) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT update.hook_run_id
         FROM agent_updates AS update
         JOIN runs AS run ON run.id = update.hook_run_id
         WHERE update.state IN ('hook_running', 'hook_committed')
           AND run.state = 'cleaned_up'
           AND run.run_kind = 'update'
         ORDER BY update.created_at, update.id",
    )
    .fetch_all(pool)
    .await
}

/// Durable update admissions that have not yet created a hook run.
///
/// The generation is derived from the update-run history for the instance so
/// a retry receives a new, deterministic run identity.
#[derive(Debug, FromRow)]
pub struct PendingUpdateAdmission {
    /// Exact update identity.
    pub update_id: Uuid,
    /// Actor captured when the update was accepted.
    pub actor_id: Uuid,
    /// Monotonic update-hook attempt generation for this instance.
    pub generation: i64,
    /// Creation timestamp used with the ID as a stable pagination cursor.
    pub created_at: OffsetDateTime,
}

/// Lists accepted draining updates that still need durable hook admission.
pub async fn pending_update_admissions(
    pool: &PgPool,
    after: Option<(OffsetDateTime, Uuid)>,
) -> Result<Vec<PendingUpdateAdmission>, sqlx::Error> {
    sqlx::query_as(
        "SELECT update.id AS update_id,
                update.actor_id,
                (SELECT count(*)
                 FROM runs AS prior_run
                 WHERE prior_run.instance_id = update.instance_id
                   AND prior_run.run_kind = 'update')::bigint AS generation
                , update.created_at
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.state = 'draining'
           AND update.hook_run_id IS NULL
           AND instance.state = 'update_draining'
           AND NOT instance.run_gate_open
           AND ($1::timestamptz IS NULL
                OR update.created_at > $1
                OR (update.created_at = $1 AND update.id > $2))
         ORDER BY update.created_at, update.id
         LIMIT 64",
    )
    .bind(after.map(|(created_at, _)| created_at))
    .bind(after.map(|(_, id)| id))
    .fetch_all(pool)
    .await
}

/// Returns whether an update-kind run is the exact hook run for an update.
pub async fn is_update_hook_run(pool: &PgPool, run_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_updates WHERE hook_run_id = $1)")
        .bind(run_id)
        .fetch_one(pool)
        .await
}
