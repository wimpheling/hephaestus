//! Candidate revision preparation for release updates.

use super::release_brokered_helpers::validate_brokered_rule_copies;
use super::{
    AuthenticatedIdentity, BrokeredRuleCloneRow, CapabilityBinding, CapabilityBindingId,
    CreateInstanceUpdate, ObjectRef, ObjectType, ParameterDeclaration, ParameterDocument,
    Permission, ReleaseService, ReleaseServiceError, RevisionBindingRow, RuntimePolicy,
    UpdateCandidateRow, UpdateCurrentRow, authorize_capability_selection,
    candidate_accepts_binding, capability_resource_is_in_project, load_capability_requirements,
    load_carried_capability_bindings, load_git_ceilings, policy_from_contract,
    required_slot_diagnostics, update_contract_diagnostics,
};
use git_capability_domain::{BoundGitCapability, RepositoryId as GitRepositoryId};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub struct PreparedUpdate {
    pub(super) publication_mode: String,
    pub(super) parameters: ParameterDocument,
    pub(super) effective: RuntimePolicy,
    pub(super) carried: Vec<RevisionBindingRow>,
    pub(super) brokered_rules: Vec<BrokeredRuleCloneRow>,
    pub(super) rule_copies: BTreeMap<Uuid, Uuid>,
    pub(super) candidate_capabilities: Vec<(CapabilityBinding, String, Option<BoundGitCapability>)>,
    pub(super) diagnostics: Vec<Value>,
    pub(super) runnable: bool,
    pub(super) publication_repository_binding_id: Option<Uuid>,
    pub(super) new_binding_ids: Vec<Uuid>,
}

// Keep the validation and binding selection in one ordered transaction phase.
#[allow(clippy::too_many_lines)]
pub async fn prepare_update(
    tx: &mut Transaction<'_, Postgres>,
    service: &ReleaseService,
    identity: &AuthenticatedIdentity,
    command: &CreateInstanceUpdate,
    current: UpdateCurrentRow,
    candidate: UpdateCandidateRow,
) -> Result<PreparedUpdate, ReleaseServiceError> {
    let declarations: Vec<ParameterDeclaration> =
        serde_json::from_value(candidate.parameter_schema)?;
    let release_policy = policy_from_contract(&candidate.runtime_contract)?;
    let effective = RuntimePolicy::resolve(
        &release_policy,
        &command.selected_policy,
        &command.platform_policy,
    )?;
    let carried: Vec<RevisionBindingRow> = sqlx::query_as(
        "SELECT binding.id, binding.import_id, binding.slot_key,
                binding.delivery_mode, binding.phases,
                binding.attachment_ids, binding.destinations,
                binding.effective_policy, binding.effective_policy_hash
         FROM agent_secret_bindings AS binding
         JOIN secret_imports AS imported ON imported.id = binding.import_id
         JOIN secret_grants AS source_grant
           ON source_grant.id = imported.grant_id
         JOIN secrets AS secret ON secret.id = imported.secret_id
         WHERE binding.instance_revision_id = $1
           AND binding.status = 'active'
           AND imported.status = 'active'
           AND source_grant.status = 'active'
           AND secret.status = 'active'
           AND (source_grant.expires_at IS NULL
                OR source_grant.expires_at > now())
         ORDER BY binding.slot_key",
    )
    .bind(command.expected_revision_id.as_uuid())
    .fetch_all(&mut **tx)
    .await?;
    let brokered_rules: Vec<BrokeredRuleCloneRow> = sqlx::query_as(
        "SELECT rule.id, rule.binding_id, secret.active_version_id AS secret_version_id,
                rule.destination_origin, rule.location_kind,
                rule.header_name, rule.header_prefix
         FROM brokered_secret_rules AS rule
         JOIN agent_secret_bindings AS binding
           ON binding.id = rule.binding_id
          AND binding.instance_revision_id = rule.instance_revision_id
         JOIN secret_imports AS imported ON imported.id = binding.import_id
         JOIN secret_grants AS source_grant
           ON source_grant.id = imported.grant_id
         JOIN secrets AS secret ON secret.id = imported.secret_id
         WHERE rule.instance_revision_id = $1
           AND binding.status = 'active'
           AND binding.delivery_mode = 'brokered'
           AND imported.status = 'active'
           AND source_grant.status = 'active'
           AND (source_grant.expires_at IS NULL
                OR source_grant.expires_at > now())
           AND secret.status = 'active'
           AND secret.active_version_id IS NOT NULL",
    )
    .bind(command.expected_revision_id.as_uuid())
    .fetch_all(&mut **tx)
    .await?;
    let parameters = ParameterDocument::resolve(&declarations, &command.parameters)
        .map_err(ReleaseServiceError::InvalidParameters)?;
    let expected: Vec<Uuid> = serde_json::from_value(current.secret_bindings)?;
    let mut diagnostics = Vec::new();
    let source_rule_ids = brokered_rules
        .iter()
        .map(|rule| rule.id)
        .collect::<BTreeSet<_>>();
    let (rule_copies, copy_diagnostics) =
        validate_brokered_rule_copies(&source_rule_ids, &command.brokered_rule_copies);
    diagnostics.extend(copy_diagnostics);
    for candidate_rule_id in rule_copies.values() {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM brokered_secret_rules WHERE id = $1
             )",
        )
        .bind(candidate_rule_id)
        .fetch_one(&mut **tx)
        .await?;
        if exists {
            diagnostics.push(json!({
                "code": "brokered_rule_copy_target_conflict",
                "field": "brokered_rule_copies"
            }));
        }
    }
    if carried.len() != expected.len() {
        diagnostics.push(json!({
            "code": "secret_binding_unavailable",
            "field": "secret_slots"
        }));
    }
    for binding in &carried {
        service
            .require(
                tx,
                identity,
                if binding.delivery_mode == "raw" {
                    Permission::BindRaw
                } else {
                    Permission::BindBrokered
                },
                ObjectRef::new(ObjectType::SecretImport, binding.import_id),
            )
            .await?;
        if !candidate_accepts_binding(&candidate.secret_slot_schema, binding) {
            diagnostics.push(json!({
                "code": "secret_binding_incompatible",
                "field": format!("secret_slots.{}", binding.slot_key)
            }));
        }
    }
    let bound_slots = carried
        .iter()
        .map(|binding| binding.slot_key.as_str())
        .collect::<std::collections::HashSet<_>>();
    diagnostics.extend(required_slot_diagnostics(
        &candidate.secret_slot_schema,
        &bound_slots,
    )?);
    diagnostics.extend(update_contract_diagnostics(
        current.requires_state,
        candidate.requires_state,
        candidate.update_hook.as_ref(),
    ));
    let carried_capabilities =
        load_carried_capability_bindings(tx, command.expected_revision_id).await?;
    let candidate_requirements =
        load_capability_requirements(tx, command.candidate_release_agent_id.as_uuid()).await?;
    let candidate_git_ceilings =
        load_git_ceilings(tx, command.candidate_release_agent_id.as_uuid()).await?;
    let mut candidate_capabilities = Vec::new();
    for carried in &carried_capabilities {
        let Some(requirement) = candidate_requirements.get(carried.binding.slot()) else {
            continue;
        };
        let Ok(binding) = CapabilityBinding::bind(
            CapabilityBindingId::new(),
            requirement,
            carried.binding.resource(),
            carried.binding.granted_operations(),
        ) else {
            diagnostics.push(json!({
                "code": "capability_binding_incompatible",
                "field": format!("capability_slots.{}", carried.binding.slot()),
            }));
            continue;
        };
        if !capability_resource_is_in_project(
            tx,
            current.project_id,
            binding.resource().kind,
            binding.resource().id,
        )
        .await?
        {
            diagnostics.push(json!({
                "code": "capability_resource_unavailable",
                "field": format!("capability_slots.{}", binding.slot()),
            }));
            continue;
        }
        authorize_capability_selection(service, tx, identity, &binding).await?;
        let git_binding = match (
            &carried.git_binding,
            candidate_git_ceilings.get(&requirement.id()),
        ) {
            (Some(source), Some(ceiling)) => Some(BoundGitCapability::new(
                GitRepositoryId::new(binding.resource().id),
                source.authority().clone(),
                ceiling,
            )?),
            (None, None) => None,
            (Some(_), None) | (None, Some(_)) => {
                diagnostics.push(json!({
                    "code": "capability_binding_incompatible",
                    "field": format!("capability_slots.{}", binding.slot()),
                }));
                continue;
            }
        };
        candidate_capabilities.push((
            binding,
            carried.authorization_model_version.clone(),
            git_binding,
        ));
    }
    diagnostics.extend(candidate_requirements.values().filter_map(|requirement| {
        let missing = requirement.slot_required()
            && !candidate_capabilities
                .iter()
                .any(|(binding, _, _)| binding.slot() == requirement.slot());
        missing.then(|| {
            json!({
                "code": "required_capability_binding_missing",
                "field": format!("capability_slots.{}", requirement.slot()),
            })
        })
    }));
    let runnable = diagnostics.is_empty();
    let publication_repository_binding_id = candidate
        .publication_repository_slot
        .as_deref()
        .and_then(|slot| {
            candidate_capabilities
                .iter()
                .find(|(binding, _, _)| binding.slot().as_str() == slot)
        })
        .map(|(binding, _, _)| binding.id().as_uuid());
    let new_binding_ids = carried.iter().map(|_| Uuid::new_v4()).collect::<Vec<_>>();
    Ok(PreparedUpdate {
        publication_mode: candidate.publication_mode.clone(),
        parameters,
        effective,
        carried,
        brokered_rules,
        rule_copies,
        candidate_capabilities,
        diagnostics,
        runnable,
        publication_repository_binding_id,
        new_binding_ids,
    })
}
