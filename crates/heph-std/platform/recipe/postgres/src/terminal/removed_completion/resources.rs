//! A terminal result cannot turn incomplete or absent owned history into success.
use crate::repository_error;
use recipe_application::{
    DeploymentError, DeploymentSnapshot, RemovalProgress, RemoveResumeConfiguration,
    ResourceOwnership,
};
use recipe_domain::RemovalPolicy;
use sqlx::{Postgres, Transaction};

pub(super) fn policy(snapshot: &DeploymentSnapshot) -> Result<(), DeploymentError> {
    if snapshot.resources.len() != snapshot.intent.resources().len() {
        return Err(DeploymentError::IntentMismatch);
    }
    for (name, plan) in snapshot.intent.resources() {
        let state = snapshot
            .resources
            .get(name)
            .ok_or(DeploymentError::IntentMismatch)?;
        let expected = match (plan.ownership(), plan.removal()) {
            (ResourceOwnership::Owned, RemovalPolicy::Delete) => RemovalProgress::Deleted,
            (_, RemovalPolicy::Retain) => RemovalProgress::Retained,
            (ResourceOwnership::External, RemovalPolicy::Delete) => {
                return Err(DeploymentError::InvalidAction);
            }
        };
        if state.active_attempt.is_some() || state.removal != expected {
            return Err(DeploymentError::ReconciliationRequired);
        }
    }
    Ok(())
}

pub(super) async fn qualify(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &DeploymentSnapshot,
    config: &RemoveResumeConfiguration,
) -> Result<(), DeploymentError> {
    policy(snapshot)?;
    // The result's protected finish guard observed all resources atomically.
    // Here each current endpoint also needs its actual disposition evidence,
    // including a saved Ready/partial retention fact or an Applied worker proof.
    let invalid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_deployment_resources resource
        WHERE resource.deployment_id=$1 AND (
          (resource.ownership='owned' AND resource.resource_kind='volume' AND resource.removal_progress='retained' AND NOT EXISTS(
            SELECT 1 FROM agent_instance_state_volumes volume WHERE volume.id=resource.resource_id AND volume.project_id=resource.project_id))
          OR (resource.ownership='owned' AND resource.resource_kind='instance' AND resource.removal_progress='retained' AND NOT EXISTS(
            SELECT 1 FROM agent_instances instance WHERE instance.id=resource.resource_id AND instance.project_id=resource.project_id AND instance.state<>'removed'))
          OR (resource.ownership='owned' AND resource.resource_kind='instance' AND resource.removal_progress='deleted' AND NOT EXISTS(
            SELECT 1 FROM agent_instances instance WHERE instance.id=resource.resource_id AND instance.project_id=resource.project_id AND instance.state='removed'))
          OR NOT (
            (resource.ownership='external' AND resource.removal_progress='retained' AND resource.install_progress='pending'
              AND resource.version=0 AND resource.execution_generation=0 AND resource.active_attempt_id IS NULL
              AND resource.last_verified_action IS NULL AND resource.diagnostic IS NULL
              AND NOT EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE deployment_id=resource.deployment_id AND resource_name=resource.resource_name))
            OR EXISTS(SELECT 1 FROM recipe_execution_transitions transition
              JOIN recipe_effect_attempts attempt ON attempt.id=transition.attempt_id
              JOIN recipe_deployment_commands command ON command.id=transition.command_id
              WHERE transition.deployment_id=resource.deployment_id AND transition.project_id=resource.project_id
                AND transition.resource_name=resource.resource_name AND transition.after_state=recipe_effect_resource_state(resource)
                AND transition.after_deployment_version<$2 AND transition.before_deployment_version+1=transition.after_deployment_version
                AND (transition.before_state->>'version')::bigint+1=(transition.after_state->>'version')::bigint
                AND octet_length(transition.fingerprint)=32
                AND attempt.deployment_id=resource.deployment_id AND attempt.project_id=resource.project_id
                AND attempt.resource_name=resource.resource_name AND attempt.generation=resource.execution_generation
                AND attempt.input_hash=resource.input_hash AND attempt.claim_json->'identity'=resource.plan_json->'identity'
                AND command.deployment_id=resource.deployment_id AND command.project_id=resource.project_id
                AND command.actor_id=transition.actor_id
                AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
                  WHERE event.id=transition.event_id AND event.occurrence_id=transition.id AND event.scope_kind='project'
                    AND event.scope_id=transition.project_id AND event.aggregate_type='project' AND event.aggregate_id=transition.project_id
                    AND event.event_type='project.changed' AND event.change_kind='updated' AND event.schema_version=1
                    AND event.actor_type='user' AND event.actor_id=transition.actor_id AND event.request_id=transition.request_id
                    AND event.cursor=transition.event_cursor AND event.aggregate_version=transition.event_aggregate_version)
                AND (
                  (transition.kind IN ('completed','reconciled') AND EXISTS(
                    SELECT 1 FROM recipe_effect_verifications proof WHERE proof.id=transition.verification_id
                      AND proof.attempt_id=attempt.id AND proof.verification_attempt_id=transition.operation_id
                      AND proof.deployment_id=resource.deployment_id AND proof.project_id=resource.project_id
                      AND proof.resource_name=resource.resource_name AND proof.generation=attempt.generation
                      AND proof.action=attempt.action AND proof.identity_json=attempt.claim_json->'identity'
                      AND proof.input_hash=attempt.input_hash AND proof.outcome='applied' AND proof.diagnostic IS NULL
                      AND proof.recorded_by='hephaestus_worker')
                    AND ((resource.ownership='owned' AND attempt.action=CASE resource.removal_progress WHEN 'deleted' THEN 'delete' WHEN 'retained' THEN 'retain' END)
                      OR (resource.ownership='external' AND resource.removal_progress='retained' AND attempt.action='verify_external')))
                  OR (resource.ownership='external' AND resource.removal_progress='retained' AND transition.kind='observation_retired'
                    AND attempt.action='verify_external' AND transition.verification_id IS NULL)
                  OR (resource.ownership='owned' AND resource.resource_kind='volume' AND resource.removal_progress='retained'
                    AND attempt.action='create' AND transition.verification_id IS NULL
                    AND ((transition.kind='creation_retained_ready' AND EXISTS(
                      SELECT 1 FROM recipe_owned_ready_retentions retained
                      JOIN volume_owned_ready_retention_closures closure ON closure.id=retained.closure_id
                      JOIN volume_owned_ready_retention_facts fact ON fact.id=retained.fact_id AND fact.closure_id=closure.id
                      JOIN volume_owned_backing_observations original ON original.id=closure.ready_observation_id
                      JOIN agent_instance_state_volumes volume ON volume.id=closure.volume_id
                      WHERE retained.transition_id=transition.id AND transition.ready_retention_fact_id=fact.id
                        AND closure.attempt_id=attempt.id AND closure.deployment_id=resource.deployment_id
                        AND closure.project_id=resource.project_id AND closure.resource_name=resource.resource_name
                        AND closure.volume_id=resource.resource_id AND volume.ready_retention_closure_id=closure.id
                        AND closure.vm_provider_namespace=$3 AND closure.vm_host_id=$4 AND resource.install_progress='ready'
                        AND fact.proof_json-'id'=original.proof_json-'id'
                        AND fact.evidence_hash=sha256(convert_to(fact.proof_json::text,'UTF8'))
                        AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
                          WHERE event.id=fact.event_id AND event.occurrence_id=fact.id AND event.scope_kind='project'
                            AND event.scope_id=resource.project_id AND event.actor_type='system' AND event.actor_id IS NULL)))
                    OR (transition.kind='creation_retained_partial' AND EXISTS(
                      SELECT 1 FROM volume_owned_birth_retirements retired
                      JOIN volume_owned_partial_receipts receipt ON receipt.id=retired.receipt_id
                      WHERE retired.transition_id=transition.id AND retired.volume_id=resource.resource_id
                        AND retired.project_id=resource.project_id AND retired.command_id=transition.command_id
                        AND retired.operation_id=transition.operation_id AND retired.actor_id=transition.actor_id
                        AND retired.request_id=transition.request_id AND retired.fingerprint=transition.fingerprint
                        AND retired.event_id=transition.event_id AND transition.partial_retention_receipt_id=receipt.id)))))
            )
          )
        ))")
        .bind(snapshot.intent.id().as_uuid()).bind(i64::try_from(snapshot.version).map_err(|_|DeploymentError::StaleClaim)?)
        .bind(config.namespace()).bind(config.host())
        .fetch_one(&mut **tx).await.map_err(repository_error)?;
    if invalid {
        Err(DeploymentError::ReconciliationRequired)
    } else {
        Ok(())
    }
}
