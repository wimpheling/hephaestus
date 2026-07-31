//! Authorized agent-instance read model.

use identity_domain::AuthenticatedIdentity;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

const MAX_REVISIONS: usize = 200;
const MAX_ATTACHMENTS: usize = 200;
const MAX_UPDATES: usize = 100;
const MAX_REPOSITORIES: usize = 200;
const MAX_IMPORTS: usize = 200;
const MAX_CANDIDATES: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum InstanceQueryError {
    #[error("agent-instance access is denied")]
    PermissionDenied,
    #[error("agent instance was not found")]
    NotFound,
    #[error("agent-instance response exceeds its bounded collection limits")]
    ResponseTooLarge,
    #[error("agent-instance query failed")]
    Persistence(#[source] sqlx::Error),
}

#[derive(FromRow)]
// These independent flags are database-projected authorization decisions.
#[allow(clippy::struct_excessive_bools)]
pub struct InstanceRow {
    pub id: Uuid,
    pub name: String,
    pub state: String,
    pub run_gate_open: bool,
    pub active_revision_id: Uuid,
    pub state_volume_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub project_id: Uuid,
    pub project_name: String,
    pub organization_id: Uuid,
    pub organization_name: String,
    pub can_manage: bool,
    pub can_update: bool,
    pub can_recover: bool,
}

#[derive(FromRow)]
pub struct RevisionRow {
    pub id: Uuid,
    pub parameters: Value,
    pub parameter_hash: Vec<u8>,
    pub resource_selection: Value,
    pub network_restriction: Value,
    pub effective_runtime_policy: Value,
    pub platform_policy_version: String,
    pub runnable: bool,
    pub diagnostics: Value,
    pub created_at: OffsetDateTime,
    pub release_agent_id: Uuid,
    pub parameter_schema: Value,
    pub secret_slot_schema: Value,
    pub runtime_contract: Value,
    pub update_hook: Option<Value>,
    pub release_id: Uuid,
    pub release_version: String,
    pub release_state: String,
    pub release_agent_name: String,
}

#[derive(FromRow)]
pub struct AttachmentRow {
    pub id: Uuid,
    pub ref_selector: String,
    pub trigger_policy: String,
    pub enabled: bool,
    pub removed_at: Option<OffsetDateTime>,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub can_manage: bool,
}

#[derive(FromRow)]
pub struct UpdateRow {
    pub id: Uuid,
    pub expected_current_revision_id: Uuid,
    pub candidate_revision_id: Uuid,
    pub state: String,
    pub hook_run_id: Option<Uuid>,
    pub hook_exit_code: Option<i32>,
    pub hook_exit_signal: Option<i32>,
    pub diagnostics: Value,
    pub final_decision: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub hook_events: Value,
}

#[derive(FromRow)]
pub struct RepositoryRow {
    pub id: Uuid,
    pub name: String,
    pub default_branch: String,
}

#[derive(FromRow)]
pub struct ImportRow {
    pub id: Uuid,
    pub alias: String,
    pub target_kind: String,
    pub target_id: Uuid,
    pub status: String,
    pub secret_name: String,
    pub secret_status: String,
    pub delivery_modes: Vec<String>,
    pub phases: Vec<String>,
    pub destinations: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
}

#[derive(FromRow)]
pub struct CandidateRow {
    pub id: Uuid,
    pub display_name: String,
    pub parameter_schema: Value,
    pub secret_slot_schema: Value,
    pub runtime_contract: Value,
    pub requires_state: bool,
    pub update_hook: Option<Value>,
    pub release_id: Uuid,
    pub release_version: String,
}

#[derive(FromRow)]
pub struct RecentRunRow {
    pub id: Uuid,
    pub state: String,
    pub outcome: Option<String>,
    pub run_kind: String,
    pub instance_revision_id: Uuid,
    pub release_id: Uuid,
    pub attachment_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub struct InstanceSnapshot {
    pub instance: InstanceRow,
    pub revisions: Vec<RevisionRow>,
    pub attachments: Vec<AttachmentRow>,
    pub updates: Vec<UpdateRow>,
    pub repositories: Vec<RepositoryRow>,
    pub imports: Vec<ImportRow>,
    pub candidates: Vec<CandidateRow>,
    pub recent_runs: Vec<RecentRunRow>,
}

pub struct InstanceApplication {
    pool: PgPool,
}

impl InstanceApplication {
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // A single transaction materializes the complete, bounded instance snapshot.
    #[allow(clippy::too_many_lines)]
    pub async fn get(
        &self,
        identity: &AuthenticatedIdentity,
        instance_id: Uuid,
    ) -> Result<InstanceSnapshot, InstanceQueryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(InstanceQueryError::Persistence)?;
        sqlx::query("SELECT set_config('hephaestus.actor_id', $1, true), set_config('hephaestus.subject_type', 'user', true), set_config('hephaestus.request_id', $2, true), set_config('hephaestus.occurrence_id', $3, true)")
            .bind(identity.user_id.to_string()).bind(identity.request_id.to_string()).bind(identity.idempotency_id.to_string()).execute(&mut *tx).await.map_err(InstanceQueryError::Persistence)?;
        let allowed: bool = sqlx::query_scalar("SELECT check_permission('user', hephaestus_actor_id(), 'can_read', 'agent_instance', $1::text) = 1")
            .bind(instance_id).fetch_one(&mut *tx).await.map_err(InstanceQueryError::Persistence)?;
        if !allowed {
            return Err(InstanceQueryError::PermissionDenied);
        }
        let instance: InstanceRow = sqlx::query_as("SELECT instance.id, instance.name, instance.state, instance.run_gate_open, instance.active_revision_id, instance.state_volume_id, instance.created_at, instance.updated_at, project.id AS project_id, project.name AS project_name, organization.id AS organization_id, organization.name AS organization_name, check_permission('user', hephaestus_actor_id(), 'can_manage', 'agent_instance', instance.id::text) = 1 AS can_manage, check_permission('user', hephaestus_actor_id(), 'can_update', 'agent_instance', instance.id::text) = 1 AS can_update, check_permission('user', hephaestus_actor_id(), 'can_recover', 'agent_instance', instance.id::text) = 1 AS can_recover FROM agent_instances instance JOIN projects project ON project.id = instance.project_id JOIN organizations organization ON organization.id = project.organization_id WHERE instance.id = $1")
            .bind(instance_id).fetch_optional(&mut *tx).await.map_err(InstanceQueryError::Persistence)?.ok_or(InstanceQueryError::NotFound)?;
        let revisions: Vec<RevisionRow> = sqlx::query_as("SELECT revision.id, revision.parameters, revision.parameter_hash, revision.resource_selection, revision.network_restriction, revision.effective_runtime_policy, revision.platform_policy_version, revision.runnable, revision.diagnostics, revision.created_at, release_agent.id AS release_agent_id, release_agent.parameter_schema, release_agent.secret_slot_schema, release_agent.runtime_contract, release_agent.update_hook, release.id AS release_id, release.version AS release_version, release.state AS release_state, release_agent.display_name AS release_agent_name FROM agent_instance_revisions revision JOIN release_agents release_agent ON release_agent.id = revision.release_agent_id JOIN releases release ON release.id = release_agent.release_id WHERE revision.instance_id = $1 ORDER BY revision.created_at DESC, revision.id LIMIT 201")
            .bind(instance_id).fetch_all(&mut *tx).await.map_err(InstanceQueryError::Persistence)?;
        let attachments: Vec<AttachmentRow> = sqlx::query_as("SELECT attachment.id, attachment.ref_selector, attachment.trigger_policy, attachment.enabled, attachment.removed_at, repository.id AS repository_id, repository.name AS repository_name, check_permission('user', hephaestus_actor_id(), 'can_manage', 'agent_attachment', attachment.id::text) = 1 AS can_manage FROM agent_attachments attachment JOIN repositories repository ON repository.id = attachment.repository_id WHERE attachment.instance_id = $1 ORDER BY repository.name, attachment.id LIMIT 201")
            .bind(instance_id).fetch_all(&mut *tx).await.map_err(InstanceQueryError::Persistence)?;
        let updates: Vec<UpdateRow> = sqlx::query_as("SELECT update_record.id, update_record.expected_current_revision_id, update_record.candidate_revision_id, update_record.state, update_record.hook_run_id, update_record.hook_exit_code, update_record.hook_exit_signal, update_record.diagnostics, update_record.final_decision, update_record.created_at, update_record.updated_at, (SELECT COALESCE(jsonb_agg(jsonb_build_object('sequence', event.sequence, 'event_type', event.event_type, 'payload', CASE WHEN event.event_type = 'vm.log' THEN jsonb_build_object('message', left(event.payload ->> 'message', 4096)) ELSE event.payload END) ORDER BY event.sequence), '[]'::jsonb) FROM (SELECT sequence, event_type, payload FROM run_events WHERE run_id = update_record.hook_run_id ORDER BY sequence LIMIT 501) event) AS hook_events FROM agent_updates update_record WHERE update_record.instance_id = $1 ORDER BY update_record.created_at DESC, update_record.id LIMIT 101")
            .bind(instance_id).fetch_all(&mut *tx).await.map_err(InstanceQueryError::Persistence)?;
        let repositories: Vec<RepositoryRow> = sqlx::query_as("SELECT id, name, default_branch FROM repositories WHERE project_id = $1 ORDER BY name, id LIMIT 201")
            .bind(instance.project_id).fetch_all(&mut *tx).await.map_err(InstanceQueryError::Persistence)?;
        let imports: Vec<ImportRow> = sqlx::query_as("SELECT secret_import.id, secret_import.alias, secret_import.target_kind, secret_import.target_id, secret_import.status, secret.name AS secret_name, secret.status AS secret_status, secret_grant.delivery_modes, secret_grant.phases, secret_grant.destinations, secret_grant.expires_at FROM secret_imports secret_import JOIN secret_grants secret_grant ON secret_grant.id = secret_import.grant_id JOIN secrets secret ON secret.id = secret_import.secret_id WHERE secret_import.status = 'active' AND secret_grant.status = 'active' AND secret.status = 'active' AND ((secret_import.target_kind = 'project' AND secret_import.target_id = $1) OR (secret_import.target_kind = 'repository' AND secret_import.target_id IN (SELECT repository_id FROM agent_attachments WHERE instance_id = $2 AND enabled AND removed_at IS NULL))) ORDER BY secret_import.alias, secret_import.id LIMIT 201")
            .bind(instance.project_id).bind(instance_id).fetch_all(&mut *tx).await.map_err(InstanceQueryError::Persistence)?;
        let candidates: Vec<CandidateRow> = sqlx::query_as("SELECT candidate.id, candidate.display_name, candidate.parameter_schema, candidate.secret_slot_schema, candidate.runtime_contract, candidate.requires_state, candidate.update_hook, release.id AS release_id, release.version AS release_version FROM agent_instance_revisions active_revision JOIN release_agents active_agent ON active_agent.id = active_revision.release_agent_id JOIN release_agents candidate ON candidate.family_id = active_agent.family_id JOIN releases release ON release.id = candidate.release_id WHERE active_revision.id = $1 AND release.state = 'published' AND candidate.id <> active_agent.id AND check_permission('user', hephaestus_actor_id(), 'can_use', 'release_agent', candidate.id::text) = 1 ORDER BY release.created_at DESC, candidate.id LIMIT 101")
            .bind(instance.active_revision_id).fetch_all(&mut *tx).await.map_err(InstanceQueryError::Persistence)?;
        let recent_runs = sqlx::query_as("SELECT id, state, outcome, run_kind, instance_revision_id, release_id, attachment_id, created_at, updated_at FROM runs WHERE instance_id = $1 ORDER BY created_at DESC, id LIMIT 20")
            .bind(instance_id).fetch_all(&mut *tx).await.map_err(InstanceQueryError::Persistence)?;
        if revisions.len() > MAX_REVISIONS
            || attachments.len() > MAX_ATTACHMENTS
            || updates.len() > MAX_UPDATES
            || repositories.len() > MAX_REPOSITORIES
            || imports.len() > MAX_IMPORTS
            || candidates.len() > MAX_CANDIDATES
        {
            return Err(InstanceQueryError::ResponseTooLarge);
        }
        tx.commit().await.map_err(InstanceQueryError::Persistence)?;
        Ok(InstanceSnapshot {
            instance,
            revisions,
            attachments,
            updates,
            repositories,
            imports,
            candidates,
            recent_runs,
        })
    }
}
