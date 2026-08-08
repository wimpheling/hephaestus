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
const MAX_CAPABILITY_REQUIREMENTS: usize = 400;
const MAX_CAPABILITY_RESOURCES: usize = 400;
const MAX_CAPABILITY_BINDINGS: usize = 400;
const MAX_RUNTIME_SESSIONS: usize = 100;
const MAX_CAPABILITY_AUDIT: usize = 200;

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

#[derive(FromRow)]
pub struct CapabilityRequirementRow {
    pub id: Uuid,
    pub release_agent_id: Uuid,
    pub slot_key: String,
    pub purpose: String,
    pub resource_kind: String,
    pub required_operations: Vec<String>,
    pub optional_operations: Vec<String>,
    pub slot_required: bool,
}

#[derive(FromRow)]
pub struct CapabilityResourceOptionRow {
    pub id: Uuid,
    pub slot_key: String,
    pub resource_kind: String,
    pub display_name: String,
    pub grantable_operations: Vec<String>,
}

#[derive(FromRow)]
pub struct CapabilityBindingRow {
    pub id: Uuid,
    pub instance_revision_id: Uuid,
    pub requirement_id: Uuid,
    pub slot_key: String,
    pub resource_kind: String,
    pub resource_id: Uuid,
    pub resource_name: String,
    pub granted_operations: Vec<String>,
    pub grantor_id: Uuid,
    pub grantor_name: String,
    pub authorization_model_version: String,
    pub created_at: OffsetDateTime,
    pub live: bool,
    pub last_used_at: Option<OffsetDateTime>,
}

#[derive(FromRow)]
pub struct RuntimeSessionRow {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub run_id: Uuid,
    pub instance_revision_id: Uuid,
    pub status: String,
    pub issued_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub acknowledged_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    pub revocation_reason: Option<String>,
}

#[derive(FromRow)]
pub struct CapabilityAuditRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub runtime_session_id: Uuid,
    pub snapshot_id: Uuid,
    pub binding_id: Uuid,
    pub slot_key: String,
    pub resource_kind: String,
    pub resource_id: Uuid,
    pub operation: String,
    pub event_kind: String,
    pub decision: Option<String>,
    pub outcome: Option<String>,
    pub reason_code: Option<String>,
    pub authorization_model_version: String,
    pub occurred_at: OffsetDateTime,
}

pub struct CapabilityMetricsRow {
    pub sessions_issued: u64,
    pub sessions_active: u64,
    pub sessions_expired: u64,
    pub sessions_revoked: u64,
    pub capability_calls: u64,
    pub ceiling_denials: u64,
    pub live_authorization_denials: u64,
    pub invalid_revisions: u64,
    pub average_revocation_latency_milliseconds: u64,
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
    pub capability_requirements: Vec<CapabilityRequirementRow>,
    pub capability_resources: Vec<CapabilityResourceOptionRow>,
    pub capability_bindings: Vec<CapabilityBindingRow>,
    pub runtime_sessions: Vec<RuntimeSessionRow>,
    pub capability_audit: Vec<CapabilityAuditRow>,
    pub capability_metrics: CapabilityMetricsRow,
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
        let capability_requirements: Vec<CapabilityRequirementRow> = sqlx::query_as(
            "SELECT requirement.id, requirement.release_agent_id,
                    requirement.slot_key, requirement.purpose,
                    requirement.resource_kind, requirement.required_operations,
                    requirement.optional_operations, requirement.slot_required
             FROM release_capability_requirements AS requirement
             WHERE requirement.release_agent_id IN (
                 SELECT revision.release_agent_id
                 FROM agent_instance_revisions AS revision
                 WHERE revision.instance_id = $1
                 UNION
                 SELECT candidate.id
                 FROM agent_instance_revisions AS active_revision
                 JOIN release_agents AS active_agent
                   ON active_agent.id = active_revision.release_agent_id
                 JOIN release_agents AS candidate
                   ON candidate.family_id = active_agent.family_id
                 JOIN releases AS release ON release.id = candidate.release_id
                 WHERE active_revision.id = $2 AND release.state = 'published'
             )
             ORDER BY requirement.release_agent_id, requirement.slot_key
             LIMIT 401",
        )
        .bind(instance_id)
        .bind(instance.active_revision_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(InstanceQueryError::Persistence)?;
        let capability_resources: Vec<CapabilityResourceOptionRow> = sqlx::query_as(
            "WITH active_requirement AS (
                 SELECT requirement.*
                 FROM agent_instance_revisions AS revision
                 JOIN release_capability_requirements AS requirement
                   ON requirement.release_agent_id = revision.release_agent_id
                 WHERE revision.id = $2
             ), resource AS (
                 SELECT repository.id, 'repository'::text AS resource_kind,
                        repository.name AS display_name
                 FROM repositories AS repository WHERE repository.project_id = $1
                 UNION ALL
                 SELECT project.id, 'project', project.name
                 FROM projects AS project WHERE project.id = $1
                 UNION ALL
                 SELECT candidate.id, 'agent_instance', candidate.name
                 FROM agent_instances AS candidate WHERE candidate.project_id = $1
                 UNION ALL
                 SELECT run.id, 'run', 'run ' || left(run.id::text, 8)
                 FROM runs AS run JOIN agent_instances AS owner
                   ON owner.id = run.instance_id WHERE owner.project_id = $1
                 UNION ALL
                 SELECT volume.id, 'state_volume',
                        'state ' || left(volume.id::text, 8)
                 FROM agent_instance_state_volumes AS volume
                 JOIN agent_instances AS owner ON owner.id = volume.instance_id
                 WHERE owner.project_id = $1
             )
             SELECT resource.id, requirement.slot_key,
                    resource.resource_kind, resource.display_name,
                    ARRAY(
                        SELECT operation
                        FROM unnest(
                            requirement.required_operations ||
                            requirement.optional_operations
                        ) AS operation
                        WHERE can_grant_agent_capability_operations(
                            hephaestus_actor_id(), resource.resource_kind,
                            resource.id, ARRAY[operation]
                        )
                        ORDER BY operation
                    ) AS grantable_operations
             FROM active_requirement AS requirement
             JOIN resource ON resource.resource_kind = requirement.resource_kind
             WHERE can_grant_agent_capability_operations(
                 hephaestus_actor_id(), resource.resource_kind, resource.id,
                 requirement.required_operations
             )
             ORDER BY requirement.slot_key, resource.display_name, resource.id
             LIMIT 401",
        )
        .bind(instance.project_id)
        .bind(instance.active_revision_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(InstanceQueryError::Persistence)?;
        let capability_bindings: Vec<CapabilityBindingRow> = sqlx::query_as(
            "SELECT binding.id, binding.instance_revision_id,
                    binding.requirement_id, binding.slot_key,
                    binding.resource_kind, binding.resource_id,
                    CASE binding.resource_kind
                      WHEN 'repository' THEN COALESCE((SELECT name FROM repositories WHERE id = binding.resource_id), 'unavailable')
                      WHEN 'project' THEN COALESCE((SELECT name FROM projects WHERE id = binding.resource_id), 'unavailable')
                      WHEN 'agent_instance' THEN COALESCE((SELECT name FROM agent_instances WHERE id = binding.resource_id), 'unavailable')
                      WHEN 'run' THEN 'run ' || left(binding.resource_id::text, 8)
                      WHEN 'state_volume' THEN 'state ' || left(binding.resource_id::text, 8)
                      ELSE 'unavailable'
                    END AS resource_name,
                    binding.granted_operations, binding.created_by AS grantor_id,
                    grantor.display_name AS grantor_name,
                    binding.authorization_model_version, binding.created_at,
                    revision.id = instance.active_revision_id
                      AND bool_and(check_permission(
                        'agent_instance', instance.id::text,
                        'agent_' || operation.name, binding.resource_kind,
                        binding.resource_id::text
                      ) = 1) AS live,
                    max(audit.occurred_at) AS last_used_at
             FROM agent_capability_bindings AS binding
             JOIN agent_instance_revisions AS revision
               ON revision.id = binding.instance_revision_id
             JOIN agent_instances AS instance ON instance.id = revision.instance_id
             JOIN users AS grantor ON grantor.id = binding.created_by
             CROSS JOIN LATERAL unnest(binding.granted_operations) AS operation(name)
             LEFT JOIN capability_audit_inspection AS audit
               ON audit.binding_id = binding.id
              AND check_permission('user', hephaestus_actor_id(), 'can_read',
                                   'run', audit.run_id::text) = 1
             WHERE revision.instance_id = $1
             GROUP BY binding.id, revision.id, instance.id,
                      instance.active_revision_id, grantor.display_name
             ORDER BY binding.created_at DESC, binding.id
             LIMIT 401",
        )
        .bind(instance_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(InstanceQueryError::Persistence)?;
        let runtime_sessions: Vec<RuntimeSessionRow> =
            sqlx::query_as("SELECT * FROM inspect_runtime_authority_sessions($1, 100)")
                .bind(instance_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(InstanceQueryError::Persistence)?;
        let capability_audit: Vec<CapabilityAuditRow> = sqlx::query_as(
            "SELECT audit.id, audit.run_id, audit.runtime_session_id,
                    audit.snapshot_id, audit.binding_id, audit.slot_key,
                    audit.resource_kind, audit.resource_id, audit.operation,
                    audit.event_kind, audit.decision, audit.outcome,
                    audit.reason_code, audit.authorization_model_version,
                    audit.occurred_at
             FROM capability_audit_inspection AS audit
             WHERE audit.instance_id = $1
               AND check_permission('user', hephaestus_actor_id(), 'can_read',
                                    'run', audit.run_id::text) = 1
             ORDER BY audit.occurred_at DESC, audit.id DESC
             LIMIT 200",
        )
        .bind(instance_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(InstanceQueryError::Persistence)?;
        let count = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
        let capability_metrics = CapabilityMetricsRow {
            sessions_issued: count(runtime_sessions.len()),
            sessions_active: count(
                runtime_sessions
                    .iter()
                    .filter(|row| row.status == "active")
                    .count(),
            ),
            sessions_expired: count(
                runtime_sessions
                    .iter()
                    .filter(|row| row.status == "expired")
                    .count(),
            ),
            sessions_revoked: count(
                runtime_sessions
                    .iter()
                    .filter(|row| row.status == "revoked")
                    .count(),
            ),
            capability_calls: count(
                capability_audit
                    .iter()
                    .filter(|row| row.event_kind == "capability_use")
                    .count(),
            ),
            ceiling_denials: count(
                capability_audit
                    .iter()
                    .filter(|row| {
                        row.decision.as_deref() == Some("deny")
                            && row.reason_code.as_deref() == Some("snapshot_ceiling_denied")
                    })
                    .count(),
            ),
            live_authorization_denials: count(
                capability_audit
                    .iter()
                    .filter(|row| {
                        row.decision.as_deref() == Some("deny")
                            && row.reason_code.as_deref() == Some("live_authorization_denied")
                    })
                    .count(),
            ),
            invalid_revisions: count(revisions.iter().filter(|row| !row.runnable).count()),
            average_revocation_latency_milliseconds: average_revocation_latency(&runtime_sessions),
        };
        if revisions.len() > MAX_REVISIONS
            || attachments.len() > MAX_ATTACHMENTS
            || updates.len() > MAX_UPDATES
            || repositories.len() > MAX_REPOSITORIES
            || imports.len() > MAX_IMPORTS
            || candidates.len() > MAX_CANDIDATES
            || capability_requirements.len() > MAX_CAPABILITY_REQUIREMENTS
            || capability_resources.len() > MAX_CAPABILITY_RESOURCES
            || capability_bindings.len() > MAX_CAPABILITY_BINDINGS
            || runtime_sessions.len() > MAX_RUNTIME_SESSIONS
            || capability_audit.len() > MAX_CAPABILITY_AUDIT
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
            capability_requirements,
            capability_resources,
            capability_bindings,
            runtime_sessions,
            capability_audit,
            capability_metrics,
        })
    }
}

fn average_revocation_latency(rows: &[RuntimeSessionRow]) -> u64 {
    let latencies = rows
        .iter()
        .filter_map(|row| row.revoked_at.map(|revoked| revoked - row.issued_at))
        .filter_map(|duration| u64::try_from(duration.whole_milliseconds()).ok())
        .collect::<Vec<_>>();
    if latencies.is_empty() {
        return 0;
    }
    latencies.iter().sum::<u64>() / u64::try_from(latencies.len()).unwrap_or(1)
}
