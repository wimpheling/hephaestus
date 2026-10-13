use crate::{
    PostgresDeploymentRepository, effect_context, effect_state,
    execution_rows::ClaimWire,
    execution_transitions::{self, Change},
    repository_error,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, DeploymentError, DeploymentLifecycle, DeploymentOperation, EffectClaim,
    InstallProgress, RemovalProgress, ResourceEffect,
};
use release_domain::ContentHash;

impl PostgresDeploymentRepository {
    /// Commits a fresh fenced claim before any provider invocation.
    ///
    /// # Errors
    /// Rejects denied access, reused attempts, stale CAS, unresolved outcomes, or illegal actions.
    pub async fn claim_resource_effect(
        &self,
        identity: &AuthenticatedIdentity,
        effect: ResourceEffect,
    ) -> Result<EffectClaim, DeploymentError> {
        if effect.provenance != AttemptProvenance::new(identity, effect.provenance.attempt_id)? {
            return Err(DeploymentError::InputConflict);
        }
        let mut context = self
            .execution_context(identity, effect.command, effect.deployment_id, false)
            .await?;
        let request_hash = request_fingerprint(&effect)?;
        let existing: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT request_fingerprint FROM recipe_effect_attempts WHERE id=$1",
        )
        .bind(effect.provenance.attempt_id.as_uuid())
        .fetch_optional(&mut *context.tx)
        .await
        .map_err(repository_error)?;
        if let Some(previous) = existing {
            return Err(if previous.as_slice() == request_hash.as_bytes() {
                DeploymentError::ReconciliationRequired
            } else {
                DeploymentError::InputConflict
            });
        }
        if context.snapshot.version != effect.expected_deployment_version {
            return Err(DeploymentError::StaleClaim);
        }
        if matches!(
            context.snapshot.lifecycle,
            DeploymentLifecycle::Installed | DeploymentLifecycle::Removed
        ) || (effect.command.operation() == DeploymentOperation::Install && context.cleanup)
            || (effect.command.operation() == DeploymentOperation::Remove && !context.cleanup)
        {
            return Err(DeploymentError::InvalidAction);
        }
        if crate::terminal::exists(&mut context.tx, effect.command).await? {
            return Err(DeploymentError::InvalidAction);
        }
        let plan = context
            .snapshot
            .intent
            .resources()
            .get(&effect.resource)
            .ok_or(DeploymentError::Unavailable)?;
        let before =
            effect_context::state(&mut context.tx, effect.deployment_id, &effect.resource).await?;
        if crate::hydration::unsigned(before.version)? != effect.expected_resource_version {
            return Err(DeploymentError::StaleClaim);
        }
        effect_state::legality(plan, &before, effect.command.operation(), effect.action)?;
        dependencies(&context.snapshot, &effect)?;
        let claim = EffectClaim {
            command: effect.command,
            deployment_id: effect.deployment_id,
            resource: effect.resource,
            identity: plan.identity(),
            input_hash: plan.input_hash(),
            resource_version: effect
                .expected_resource_version
                .checked_add(1)
                .ok_or(DeploymentError::StaleClaim)?,
            generation: crate::hydration::unsigned(before.generation)?
                .checked_add(1)
                .ok_or(DeploymentError::StaleClaim)?,
            action: effect.action,
            provenance: effect.provenance,
        };
        let after = before.claim(&claim)?;
        insert_attempt(
            &mut context,
            identity,
            &claim,
            &before,
            effect.expected_deployment_version,
            request_hash,
        )
        .await?;
        let id =
            execution_transitions::transition_id(claim.provenance.attempt_id.as_uuid(), "claimed");
        execution_transitions::commit(
            &mut context,
            identity,
            Change {
                id,
                kind: "claimed",
                operation_id: claim.provenance.attempt_id.as_uuid(),
                fingerprint: request_hash,
                claim: &claim,
                before: &before,
                after: &after,
                proof: None,
            },
        )
        .await?;
        context.tx.commit().await.map_err(repository_error)?;
        Ok(claim)
    }
}

fn request_fingerprint(effect: &ResourceEffect) -> Result<ContentHash, DeploymentError> {
    let input = serde_json::to_vec(&(
        effect.command,
        effect.deployment_id,
        &effect.resource,
        effect.expected_deployment_version,
        effect.expected_resource_version,
        effect.action,
        effect.provenance,
    ))
    .map_err(|_| DeploymentError::Serialization)?;
    Ok(ContentHash::digest(&input))
}

async fn insert_attempt(
    context: &mut effect_context::Context<'_>,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
    before: &effect_state::State,
    expected_version: u64,
    request_hash: ContentHash,
) -> Result<(), DeploymentError> {
    sqlx::query("INSERT INTO recipe_effect_attempts (id, command_id, deployment_id, project_id, resource_name,
            action, generation, input_hash, claim_json, before_state, request_fingerprint, expected_deployment_version,
            actor_id, request_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)")
            .bind(claim.provenance.attempt_id.as_uuid()).bind(claim.command.id().as_uuid()).bind(claim.deployment_id.as_uuid())
            .bind(context.snapshot.intent.project_id().as_uuid()).bind(claim.resource.as_str()).bind(claim.action.as_str())
            .bind(i64::try_from(claim.generation).map_err(|_| DeploymentError::StaleClaim)?).bind(claim.input_hash.as_bytes().as_slice()).bind(sqlx::types::Json(ClaimWire::new(claim)?))
            .bind(sqlx::types::Json(before)).bind(request_hash.as_bytes().as_slice())
            .bind(i64::try_from(expected_version).map_err(|_| DeploymentError::StaleClaim)?)
            .bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid())
            .execute(&mut *context.tx).await.map_err(repository_error)?;
    Ok(())
}

fn dependencies(
    snapshot: &recipe_application::DeploymentSnapshot,
    effect: &ResourceEffect,
) -> Result<(), DeploymentError> {
    let plan = snapshot
        .intent
        .resources()
        .get(&effect.resource)
        .ok_or(DeploymentError::Unavailable)?;
    let ready = if effect.command.operation() == DeploymentOperation::Install {
        plan.dependencies().iter().all(|name| {
            snapshot.resources.get(name).is_some_and(|progress| {
                progress.install == InstallProgress::Ready && progress.active_attempt.is_none()
            })
        })
    } else {
        snapshot
            .intent
            .resources()
            .iter()
            .filter(|(_, candidate)| candidate.dependencies().contains(&effect.resource))
            .all(|(name, _)| {
                snapshot.resources.get(name).is_some_and(|progress| {
                    matches!(
                        progress.removal,
                        RemovalProgress::Retained | RemovalProgress::Deleted
                    ) && progress.active_attempt.is_none()
                })
            })
    };
    if ready {
        Ok(())
    } else {
        Err(DeploymentError::InvalidAction)
    }
}
