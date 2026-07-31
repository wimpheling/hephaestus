//! `PostgreSQL` launch authorization adapter.
use async_trait::async_trait;
use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision};
use identity_domain::{RequestId, UserId};
use run_domain::Run;
use run_orchestrator::RunLaunchAuthorizer;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use std::sync::Arc;
use uuid::Uuid;

/// Launch authorization backed by exact persisted run provenance.
pub struct PgRunLaunchAuthorizer {
    pool: PgPool,
    authorizer: Arc<PostgresMelangeAuthorizer>,
}

pub async fn recoverable_update_runs(pool: &PgPool) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT update.hook_run_id FROM agent_updates AS update JOIN runs AS run ON run.id = update.hook_run_id WHERE update.state IN ('hook_running','hook_committed') AND run.state = 'cleaned_up' ORDER BY update.created_at, update.id").fetch_all(pool).await
}
pub use recoverable_update_runs as recoverable_update_hook_run_ids;
pub use vm_spec_contract as load_vm_launch_contract;

pub async fn vm_spec_contract(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<
    Option<(
        Value,
        Value,
        bool,
        Option<Value>,
        String,
        bool,
        bool,
        Option<Uuid>,
    )>,
    sqlx::Error,
> {
    sqlx::query_as("SELECT release_agent.runtime_contract, revision.effective_runtime_policy, release_agent.requires_state, release_agent.update_hook, release.state, revision.runnable, ((run.run_kind = 'update' AND instance.state = 'updating') OR (run.run_kind = 'normal' AND instance.state IN ('active','update_rejected') AND attachment.enabled AND attachment.removed_at IS NULL)), attachment.id FROM runs run JOIN agent_instances instance ON instance.id=run.instance_id JOIN agent_instance_revisions revision ON revision.id=instance.active_revision_id JOIN release_agents release_agent ON release_agent.id=revision.release_agent_id JOIN releases release ON release.id=release_agent.release_id LEFT JOIN agent_attachments attachment ON attachment.id=run.attachment_id WHERE run.id=$1").bind(run_id).fetch_optional(pool).await
}
impl PgRunLaunchAuthorizer {
    /// Creates an authorization adapter.
    #[must_use]
    pub const fn new(pool: PgPool, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        Self { pool, authorizer }
    }
}
#[derive(FromRow)]
struct LaunchAuthorizationRow {
    actor_id: Option<Uuid>,
    request_id: Uuid,
    run_kind: String,
    instance_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Option<Uuid>,
}
#[async_trait]
impl RunLaunchAuthorizer for PgRunLaunchAuthorizer {
    async fn authorize(&self, run: &Run) -> Result<(), run_orchestrator::RunAuthorizationError> {
        let mut tx = self.pool.begin().await.map_err(error)?;
        let row = sqlx::query_as::<_, LaunchAuthorizationRow>("SELECT COALESCE(request.actor_id, update.actor_id) AS actor_id, COALESCE(request.request_id, update.id, run.command_id) AS request_id, run.run_kind, run.instance_id, run.release_agent_id, run.attachment_id FROM runs AS run LEFT JOIN run_requests AS request ON request.run_id = run.id LEFT JOIN agent_updates AS update ON update.hook_run_id = run.id WHERE run.id = $1")
            .bind(run.id.as_uuid()).fetch_optional(&mut *tx).await.map_err(error)?.ok_or_else(|| redacted("exact launch provenance is unavailable"))?;
        let Some(actor_id) = row.actor_id else {
            return Err(redacted("launch requester is unavailable"));
        };
        sqlx::query("SELECT set_config('hephaestus.actor_id', $1, true), set_config('hephaestus.subject_type', 'user', true), set_config('hephaestus.request_id', $2, true)").bind(actor_id.to_string()).bind(row.request_id.to_string()).execute(&mut *tx).await.map_err(error)?;
        let actor = UserId::from_uuid(actor_id);
        let request_id = RequestId::from_uuid(row.request_id);
        let mut required = vec![(
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, row.release_agent_id),
        )];
        if row.run_kind == "normal" {
            required.push((
                Permission::CanExecute,
                ObjectRef::new(
                    ObjectType::AgentAttachment,
                    row.attachment_id
                        .ok_or_else(|| redacted("normal run attachment is unavailable"))?,
                ),
            ));
        } else if row.run_kind == "update" {
            required.push((
                Permission::CanUpdate,
                ObjectRef::new(ObjectType::AgentInstance, row.instance_id),
            ));
        } else {
            return Err(redacted("run kind is invalid"));
        }
        for (permission, object) in required {
            let decision = self
                .authorizer
                .check(&mut tx, Subject::User(actor), permission, object)
                .await
                .map_err(error)?;
            audit_decision(&mut tx, actor, permission, object, decision, request_id)
                .await
                .map_err(error)?;
            if !decision.is_allowed() {
                tx.commit().await.map_err(error)?;
                return Err(redacted("live launch permission was denied"));
            }
        }
        tx.commit().await.map_err(error)?;
        Ok(())
    }
}
fn redacted(message: &str) -> run_orchestrator::RunAuthorizationError {
    run_orchestrator::RunAuthorizationError::redacted(message)
}
fn error(error: impl std::fmt::Display) -> run_orchestrator::RunAuthorizationError {
    tracing::warn!(%error, "launch authorization failed closed");
    redacted("authorization provider unavailable")
}
