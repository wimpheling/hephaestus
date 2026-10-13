use super::{
    Context, Mode, RecipeRemovalError, Request,
    admission::Current,
    retained::{Original, Volume},
};
use crate::recipe::ready_retention::ReadyRetentionHandover;
use capability_domain::CapabilitySlotKey;
use recipe_application::{
    DeploymentAttemptId, OwnedReadyRemoveResume, OwnedReadyRetentionRepository, RemovalProgress,
    RetainOwnedReady,
};
use recipe_application::{
    DeploymentError, DeploymentSnapshot, EffectClaim, PlannedResourceIdentity,
    PreparedOwnedVolumeCreation, ResourceProgress,
};
use recipe_domain::ResolvedResource;
use uuid::Uuid;
use volume_trait::{
    OwnedBackingPhase, OwnedProvisioningClaim, OwnedProvisioningContext,
    OwnedProvisioningDiscovery, OwnedProvisioningExpectation, VolumeOwnedProvisioningRepository,
};
use volume_trait::{OwnedVolumeRegistration, VolumeRegistration};

pub(super) async fn finish(
    context: &Context,
    request: &Request,
    original: &mut Original,
    current: &Current,
    resource: &CapabilitySlotKey,
) -> Result<(), RecipeRemovalError> {
    context.controller.validate_owners()?;
    let (target, prepared, discovered) = load_original(context, request, current, resource).await?;
    let progress = current
        .snapshot
        .resources
        .get(resource)
        .ok_or(RecipeRemovalError::Unsupported)?;
    let held = context
        .retention
        .pending_retention()
        .into_iter()
        .find(|pending| {
            pending.volume_id
                == discovered
                    .claim
                    .purpose
                    .receipt()
                    .intent
                    .registration()
                    .id()
                    .as_uuid()
        });
    if cold_retained(progress.removal, held.is_some()) {
        // Cold SAME-original or fresh-manager positive completed replay has no new physical guard,
        // association, Retain or proof. Actual current actor port + Source8 +
        // original worker discovery qualify it; no prior actor credentials.
        cold_completed(context, request, &target, progress).await?;
        return Ok(());
    }
    let (retained, claim) = stable_retention(
        original,
        request,
        (current.snapshot.version, progress.version),
        resource,
        &target,
        &prepared,
        &discovered.claim,
    )?;
    // Stable operation/full claim/CAS are retained ABOVE before any association,
    // physical probe, fact or actor-completion wait.
    let finished = if held
        .as_ref()
        .is_some_and(|pending| pending.operation_id != retained.operation_id)
    {
        if request.mode != Mode::Resume {
            return Err(RecipeRemovalError::Unsupported);
        }
        let fact = if progress.removal == RemovalProgress::Retained {
            Some(
                context
                    .deployment
                    .load_owned_volume_retained_fact_id(
                        &request.identity,
                        request.remove.command,
                        &current.snapshot.intent,
                        resource,
                    )
                    .await?,
            )
        } else {
            None
        };
        context
            .retention
            .handover(
                &request.identity,
                &retained,
                &claim,
                ReadyRetentionHandover {
                    remove: request.remove,
                    original: target,
                    preparation: prepared,
                    progress: progress.clone(),
                    committed_fact: fact,
                },
            )
            .await?
    } else {
        if request.mode == Mode::Resume {
            context
                .retention_actor
                .resume_owned_ready_remove(&request.identity, &request.remove, &target)
                .await?;
        }
        context
            .retention
            .retain(&request.identity, &retained, &claim)
            .await?
    };
    if finished.removal != RemovalProgress::Retained || finished.active_attempt.is_some() {
        return Err(RecipeRemovalError::Unsupported);
    }
    context.controller.validate_owners()?;
    Ok(())
}

async fn load_original(
    context: &Context,
    request: &Request,
    current: &Current,
    resource: &CapabilitySlotKey,
) -> Result<
    (
        EffectClaim,
        PreparedOwnedVolumeCreation,
        Box<OwnedProvisioningContext>,
    ),
    RecipeRemovalError,
> {
    let (target, prepared) = context
        .deployment
        .load_owned_volume_remove_original(
            &request.identity,
            request.remove.command,
            &current.snapshot.intent,
            resource,
        )
        .await?;
    let registration = registration(&current.snapshot, &target, &prepared)?;
    let owner = &context.controller.drain_inputs().volume_owner;
    let expected = OwnedProvisioningExpectation::new(
        registration,
        owner.host().to_owned(),
        owner.root().to_path_buf(),
        owner.namespace_id(),
    )?;
    let OwnedProvisioningDiscovery::Original(discovered) = context
        .volumes
        .discover_owned_provisioning(&expected)
        .await?
    else {
        return Err(RecipeRemovalError::Unsupported);
    };
    let observed = discovered
        .observation
        .as_ref()
        .ok_or(RecipeRemovalError::Unsupported)?;
    if discovered.claim.operation_id != expected.first_operation_id()
        || discovered.claim.generation != 1
        || observed.fields().phase != OwnedBackingPhase::Ready
    {
        return Err(RecipeRemovalError::Unsupported);
    }
    Ok((target, prepared, discovered))
}

async fn cold_completed(
    context: &Context,
    request: &Request,
    target: &EffectClaim,
    progress: &ResourceProgress,
) -> Result<(), RecipeRemovalError> {
    let OwnedReadyRemoveResume::Retained(saved) = context
        .retention_actor
        .resume_owned_ready_remove(&request.identity, &request.remove, target)
        .await?
    else {
        return Err(RecipeRemovalError::Unsupported);
    };
    if saved != *progress
        || saved.removal != RemovalProgress::Retained
        || saved.active_attempt.is_some()
    {
        return Err(RecipeRemovalError::Unsupported);
    }
    context.controller.validate_owners()?;
    Ok(())
}

fn stable_retention(
    original: &mut Original,
    request: &Request,
    versions: (u64, u64),
    resource: &CapabilitySlotKey,
    target: &EffectClaim,
    prepared: &PreparedOwnedVolumeCreation,
    claim: &OwnedProvisioningClaim,
) -> Result<(RetainOwnedReady, OwnedProvisioningClaim), RecipeRemovalError> {
    let entries = original.volumes.entry(resource.clone()).or_default();
    let index = if let Some(index) = entries
        .iter()
        .position(|entry| entry.request.command == request.remove.command)
    {
        if entries[index].request.target != *target
            || entries[index].preparation != *prepared
            || entries[index].claim != *claim
        {
            return Err(recipe_application::DeploymentError::InputConflict.into());
        }
        index
    } else {
        let retention = RetainOwnedReady {
            command: request.remove.command,
            target: target.clone(),
            operation_id: DeploymentAttemptId::from_uuid(Uuid::new_v4())?,
            expected_deployment_version: versions.0,
            expected_resource_version: versions.1,
        };
        entries.push(Volume {
            request: retention,
            preparation: prepared.clone(),
            claim: claim.clone(),
        });
        entries.len() - 1
    };
    let retained = entries[index].request.clone();
    let claim = entries[index].claim.clone();
    Ok((retained, claim))
}

fn registration(
    snapshot: &DeploymentSnapshot,
    claim: &EffectClaim,
    mapped: &PreparedOwnedVolumeCreation,
) -> Result<OwnedVolumeRegistration, DeploymentError> {
    let plan = snapshot
        .intent
        .resources()
        .get(&claim.resource)
        .ok_or(DeploymentError::Unavailable)?;
    let (
        PlannedResourceIdentity::Volume {
            id,
            filesystem_uuid: Some(filesystem_uuid),
        },
        ResolvedResource::Volume(volume),
    ) = (claim.identity, plan.intent())
    else {
        return Err(DeploymentError::InvalidAction);
    };
    let fields = mapped.creation.fields();
    let mut tagged = b"hephaestus-volume-effect-v1\0".to_vec();
    tagged.extend_from_slice(mapped.claim_digest.as_bytes());
    let effect = Uuid::new_v5(&claim.provenance.attempt_id.as_uuid(), &tagged);
    if mapped.mapping_version != 1
        || mapped.volume_id != id
        || mapped.filesystem_uuid != filesystem_uuid
        || mapped.project_id != snapshot.intent.project_id().as_uuid()
        || mapped.capacity_bytes != volume.capacity_bytes
        || fields.command_id != claim.command.id().as_uuid()
        || fields.attempt_id != claim.provenance.attempt_id.as_uuid()
        || fields.actor_id != claim.provenance.actor_id.as_uuid()
        || fields.request_id != claim.provenance.request_id.as_uuid()
        || fields.resource_version != claim.resource_version
        || fields.generation != claim.generation
        || fields.input_hash != *claim.input_hash.as_bytes()
        || fields.intent_hash != *snapshot.intent.input_hash().as_bytes()
        || fields.scope_id.as_uuid()
            != Uuid::new_v5(
                &claim.deployment_id.as_uuid(),
                b"hephaestus-volume-owner-scope-v1\0",
            )
        || fields.effect_id != effect
        || fields.operation_id.as_uuid()
            != Uuid::new_v5(&effect, b"hephaestus-volume-register-v1\0")
    {
        return Err(DeploymentError::InputConflict);
    }
    Ok(OwnedVolumeRegistration::new(
        VolumeRegistration::new(
            id,
            mapped.project_id,
            mapped.capacity_bytes,
            filesystem_uuid,
        )
        .map_err(|_| DeploymentError::InputConflict)?,
        mapped.creation.clone(),
    ))
}

// Both entry modes need this positive actor/worker replay; no new retention
// operation can adopt the old same-command closure after a cold restart.
fn cold_retained(removal: RemovalProgress, held: bool) -> bool {
    removal == RemovalProgress::Retained && !held
}

#[cfg(test)]
mod cold_replay_tests {
    use super::cold_retained;
    use recipe_application::RemovalProgress;

    #[test]
    fn cold_committed_replay_is_mode_independent_and_does_not_replace_held_custody() {
        assert!(cold_retained(RemovalProgress::Retained, false));
        assert!(!cold_retained(RemovalProgress::Retained, true));
        for state in [RemovalProgress::Pending, RemovalProgress::RecoveryRequired] {
            assert!(!cold_retained(state, false));
        }
    }
}
