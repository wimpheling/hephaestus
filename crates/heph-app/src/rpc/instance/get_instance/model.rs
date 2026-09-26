use super::{
    contract::{
        contract, diagnostics, events, network_restriction, parameter_schema, parameters, policy,
        recovery, secret_slots, selector, trigger, update_hook,
    },
    policy::{authority, delivery, encode_hex, opaque, phase, secret_state, target, timestamp},
};
use crate::application::instance::InstanceSnapshot;
use crate::rpc::RpcError;
use rpc_proto::messages::hephaestus::{
    instance::v1::{
        AgentInstance, AgentUpdate, Attachment, CapabilityAuditRecord, CapabilityBinding,
        CapabilityMetrics, CapabilityRequirement, CapabilityResourceOption,
        MailboxDeliveryInspection, RecentRun, RepositoryOption, RuntimeAuthoritySession,
        SecretImport, UpdateCandidate,
    },
    secret::v1::SecretPolicy,
};

// The generated response intentionally projects the complete bounded instance snapshot.
#[allow(clippy::too_many_lines)]
pub(super) fn project(snapshot: InstanceSnapshot) -> Result<AgentInstance, RpcError> {
    let capability_requirements = snapshot
        .capability_requirements
        .into_iter()
        .map(|row| CapabilityRequirement {
            id: opaque(row.id).into(),
            release_agent_id: opaque(row.release_agent_id).into(),
            slot_key: row.slot_key,
            purpose: row.purpose,
            resource_kind: row.resource_kind,
            required_operations: row.required_operations,
            optional_operations: row.optional_operations,
            slot_required: row.slot_required,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let revisions = snapshot
        .revisions
        .into_iter()
        .map(|row| {
            Ok(
                rpc_proto::messages::hephaestus::instance::v1::InstanceRevision {
                    id: opaque(row.id).into(),
                    parameters: parameters(&row.parameters)?,
                    parameter_hash: encode_hex(&row.parameter_hash),
                    resource_selection: policy(&row.resource_selection)?.into(),
                    network_restriction: network_restriction(&row.network_restriction)?.into(),
                    effective_runtime_policy: policy(&row.effective_runtime_policy)?.into(),
                    platform_policy_version: row.platform_policy_version.clone(),
                    runnable: row.runnable,
                    diagnostics: diagnostics(&row.diagnostics)?,
                    created_at: timestamp(row.created_at).into(),
                    release_agent_id: opaque(row.release_agent_id).into(),
                    parameter_schema: parameter_schema(&row.parameter_schema)?,
                    secret_slot_schema: secret_slots(&row.secret_slot_schema)?,
                    runtime_contract: contract(
                        &row.runtime_contract,
                        &row.platform_policy_version,
                    )?
                    .into(),
                    update_hook: update_hook(row.update_hook.as_ref()).into(),
                    release_id: opaque(row.release_id).into(),
                    release_version: row.release_version,
                    release_state: row.release_state,
                    release_agent_name: row.release_agent_name,
                    ..Default::default()
                },
            )
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let attachments = snapshot
        .attachments
        .into_iter()
        .map(|row| {
            Ok(Attachment {
                id: opaque(row.id).into(),
                ref_selector: selector(&row.ref_selector).into(),
                trigger_policy: trigger(&row.trigger_policy)?.into(),
                enabled: row.enabled,
                removed_at: row.removed_at.map(timestamp).into(),
                repository_id: opaque(row.repository_id).into(),
                repository_name: row.repository_name,
                can_manage: row.can_manage,
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let updates = snapshot
        .updates
        .into_iter()
        .map(|row| {
            Ok(AgentUpdate {
                id: opaque(row.id).into(),
                expected_current_revision_id: opaque(row.expected_current_revision_id).into(),
                candidate_revision_id: opaque(row.candidate_revision_id).into(),
                state: row.state,
                hook_run_id: row.hook_run_id.map(opaque).into(),
                hook_exit_code: row.hook_exit_code,
                hook_exit_signal: row.hook_exit_signal,
                diagnostics: diagnostics(&row.diagnostics)?,
                final_decision: recovery(row.final_decision.as_deref()).into(),
                created_at: timestamp(row.created_at).into(),
                updated_at: timestamp(row.updated_at).into(),
                hook_events: events(&row.hook_events)?,
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let repositories = snapshot
        .repositories
        .into_iter()
        .map(|row| RepositoryOption {
            id: opaque(row.id).into(),
            name: row.name,
            default_branch: row.default_branch,
            ..Default::default()
        })
        .collect();
    let secret_imports = snapshot
        .imports
        .into_iter()
        .map(|row| {
            Ok(SecretImport {
                id: opaque(row.id).into(),
                r#alias: row.alias,
                target: target(&row.target_kind, row.target_id)?.into(),
                state: authority(&row.status)?.into(),
                secret_name: row.secret_name,
                secret_state: secret_state(&row.secret_status)?.into(),
                policy: SecretPolicy {
                    delivery_modes: row
                        .delivery_modes
                        .iter()
                        .map(|value| delivery(value))
                        .collect::<Result<Vec<_>, _>>()?,
                    phases: row
                        .phases
                        .iter()
                        .map(|value| phase(value))
                        .collect::<Result<Vec<_>, _>>()?,
                    destinations: row.destinations,
                    ..Default::default()
                }
                .into(),
                expires_at: row.expires_at.map(timestamp).into(),
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let update_candidates = snapshot
        .candidates
        .into_iter()
        .map(|row| {
            Ok(UpdateCandidate {
                id: opaque(row.id).into(),
                display_name: row.display_name,
                parameter_schema: parameter_schema(&row.parameter_schema)?,
                secret_slot_schema: secret_slots(&row.secret_slot_schema)?,
                runtime_contract: contract(&row.runtime_contract, "")?.into(),
                requires_state: row.requires_state,
                update_hook: update_hook(row.update_hook.as_ref()).into(),
                release_id: opaque(row.release_id).into(),
                release_version: row.release_version,
                capability_requirements: capability_requirements
                    .iter()
                    .filter(|requirement| {
                        requirement
                            .release_agent_id
                            .as_option()
                            .is_some_and(|id| id.value == row.id.to_string())
                    })
                    .cloned()
                    .collect(),
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let recent_runs = snapshot
        .recent_runs
        .into_iter()
        .map(|row| RecentRun {
            id: opaque(row.id).into(),
            state: row.state,
            outcome: row.outcome.unwrap_or_default(),
            run_kind: row.run_kind,
            instance_revision_id: opaque(row.instance_revision_id).into(),
            release_id: opaque(row.release_id).into(),
            attachment_id: row.attachment_id.map(opaque).into(),
            created_at: timestamp(row.created_at).into(),
            updated_at: timestamp(row.updated_at).into(),
            ..Default::default()
        })
        .collect();
    let capability_resource_options = snapshot
        .capability_resources
        .into_iter()
        .map(|row| CapabilityResourceOption {
            id: opaque(row.id).into(),
            slot_key: row.slot_key,
            resource_kind: row.resource_kind,
            display_name: row.display_name,
            grantable_operations: row.grantable_operations,
            ..Default::default()
        })
        .collect();
    let capability_bindings = snapshot
        .capability_bindings
        .into_iter()
        .map(|row| CapabilityBinding {
            id: opaque(row.id).into(),
            instance_revision_id: opaque(row.instance_revision_id).into(),
            requirement_id: opaque(row.requirement_id).into(),
            slot_key: row.slot_key,
            resource_kind: row.resource_kind,
            resource_id: opaque(row.resource_id).into(),
            resource_name: row.resource_name,
            granted_operations: row.granted_operations,
            grantor_id: opaque(row.grantor_id).into(),
            grantor_name: row.grantor_name,
            authorization_model_version: row.authorization_model_version,
            created_at: timestamp(row.created_at).into(),
            live: row.live,
            last_used_at: row.last_used_at.map(timestamp).into(),
            ..Default::default()
        })
        .collect();
    let runtime_sessions = snapshot
        .runtime_sessions
        .into_iter()
        .map(|row| RuntimeAuthoritySession {
            id: opaque(row.id).into(),
            run_id: opaque(row.run_id).into(),
            instance_revision_id: opaque(row.instance_revision_id).into(),
            snapshot_id: opaque(row.snapshot_id).into(),
            status: row.status,
            issued_at: timestamp(row.issued_at).into(),
            expires_at: timestamp(row.expires_at).into(),
            acknowledged_at: row.acknowledged_at.map(timestamp).into(),
            revoked_at: row.revoked_at.map(timestamp).into(),
            revocation_reason: row.revocation_reason.unwrap_or_default(),
            ..Default::default()
        })
        .collect();
    let capability_audit = snapshot
        .capability_audit
        .into_iter()
        .map(|row| CapabilityAuditRecord {
            id: opaque(row.id).into(),
            run_id: opaque(row.run_id).into(),
            runtime_session_id: opaque(row.runtime_session_id).into(),
            snapshot_id: opaque(row.snapshot_id).into(),
            binding_id: opaque(row.binding_id).into(),
            slot_key: row.slot_key,
            resource_kind: row.resource_kind,
            resource_id: opaque(row.resource_id).into(),
            operation: row.operation,
            event_kind: row.event_kind,
            decision: row.decision.unwrap_or_default(),
            outcome: row.outcome.unwrap_or_default(),
            reason_code: row.reason_code.unwrap_or_default(),
            authorization_model_version: row.authorization_model_version,
            occurred_at: timestamp(row.occurred_at).into(),
            ..Default::default()
        })
        .collect();
    let mailbox_deliveries = snapshot
        .mailbox_deliveries
        .into_iter()
        .map(|row| {
            Ok(MailboxDeliveryInspection {
                event_id: opaque(row.event_id).into(),
                mailbox_id: opaque(row.mailbox_id).into(),
                disposition: row.disposition,
                logical_attempt_count: u32::try_from(row.logical_attempt_count)
                    .map_err(|_| RpcError::Internal)?,
                denial_code: row.denial_code.unwrap_or_default(),
                instance_revision_id: row.instance_revision_id.map(opaque).into(),
                state_volume_id: row.state_volume_id.map(opaque).into(),
                lease_id: row.lease_id.map(opaque).into(),
                lease_fencing_token: row
                    .lease_fencing_token
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| RpcError::Internal)?
                    .unwrap_or_default(),
                dispatch_sequence: row
                    .dispatch_sequence
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| RpcError::Internal)?
                    .unwrap_or_default(),
                state_access_outcome: row.state_access_outcome.unwrap_or_default(),
                next_eligible_at: row.next_eligible_at.map(timestamp).into(),
                next_recovery_action: row.next_recovery_action,
                updated_at: timestamp(row.updated_at).into(),
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let metrics = snapshot.capability_metrics;
    let capability_metrics = CapabilityMetrics {
        sessions_issued: metrics.sessions_issued,
        sessions_active: metrics.sessions_active,
        sessions_expired: metrics.sessions_expired,
        sessions_revoked: metrics.sessions_revoked,
        capability_calls: metrics.capability_calls,
        ceiling_denials: metrics.ceiling_denials,
        live_authorization_denials: metrics.live_authorization_denials,
        invalid_revisions: metrics.invalid_revisions,
        average_revocation_latency_milliseconds: metrics.average_revocation_latency_milliseconds,
        ..Default::default()
    };
    let row = snapshot.instance;
    Ok(AgentInstance {
        id: opaque(row.id).into(),
        name: row.name,
        state: row.state,
        run_gate_open: row.run_gate_open,
        active_revision_id: opaque(row.active_revision_id).into(),
        state_volume_id: row.state_volume_id.map(opaque).into(),
        created_at: timestamp(row.created_at).into(),
        updated_at: timestamp(row.updated_at).into(),
        project_id: opaque(row.project_id).into(),
        project_name: row.project_name,
        organization_id: opaque(row.organization_id).into(),
        organization_name: row.organization_name,
        can_manage: row.can_manage,
        can_update: row.can_update,
        can_recover: row.can_recover,
        revisions,
        attachments,
        updates,
        repositories,
        secret_imports,
        update_candidates,
        recent_runs,
        capability_requirements,
        capability_resource_options,
        capability_bindings,
        runtime_sessions,
        capability_audit,
        capability_metrics: capability_metrics.into(),
        mailbox_deliveries,
        ..Default::default()
    })
}
