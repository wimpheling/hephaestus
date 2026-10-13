use heph_runtime::VmProvider;
use recipe_application::{
    DeploymentError, DeploymentOperation, EffectClaim, PlannedResourceIdentity, ResourceAction,
    RetainedExternalVerificationPlan,
};
use volume_trait::{
    OwnedProvisioningExpectation, OwnedRetainedReuseContext, OwnedVolumeRegistration,
    VolumeRegistration,
};

pub fn plan(
    provider: &dyn VmProvider,
    plan: &RetainedExternalVerificationPlan,
    claim: &EffectClaim,
) -> Result<(), DeploymentError> {
    let scope = provider
        .owner_scope()
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    let original = &plan.preparation;
    let PlannedResourceIdentity::Volume {
        id,
        filesystem_uuid,
    } = claim.identity
    else {
        return Err(DeploymentError::InvalidAction);
    };
    if plan.claim != *claim
        || claim.command.operation() != DeploymentOperation::Install
        || claim.action != ResourceAction::VerifyExternal
        || plan.closure_id.is_nil()
        || original.mapping_version != 1
        || original.volume_id != id
        || filesystem_uuid.is_some_and(|expected| expected != original.filesystem_uuid)
        || plan.vm_provider_namespace != scope.namespace()
        || plan.vm_host_id != scope.host_id()
    {
        return Err(DeploymentError::InputConflict);
    }
    Ok(())
}

pub fn registration(
    plan: &RetainedExternalVerificationPlan,
) -> Result<OwnedVolumeRegistration, volume_trait::VolumeError> {
    let original = &plan.preparation;
    Ok(OwnedVolumeRegistration::new(
        VolumeRegistration::new(
            original.volume_id,
            original.project_id,
            original.capacity_bytes,
            original.filesystem_uuid,
        )?,
        original.creation.clone(),
    ))
}

pub fn context(
    plan: &RetainedExternalVerificationPlan,
    expected: &OwnedProvisioningExpectation,
    context: &OwnedRetainedReuseContext,
) -> Result<(), volume_trait::VolumeError> {
    context.validate()?;
    let claim = &context.retained.original.claim;
    let closure = context.retained.closure.fields();
    if claim.purpose != expected.purpose(claim.purpose.receipt().clone())?
        || claim.operation_id != expected.first_operation_id()
        || claim.generation != 1
        || closure.id != plan.closure_id
        || closure.vm_provider_namespace != plan.vm_provider_namespace
        || closure.vm_host_id != plan.vm_host_id
        || context.retention_operation_id != plan.retention_operation_id.as_uuid()
    {
        return Err(volume_trait::VolumeError::IntentConflict);
    }
    Ok(())
}

pub fn unchanged(
    original: &RetainedExternalVerificationPlan,
    current: &RetainedExternalVerificationPlan,
) -> Result<(), DeploymentError> {
    let mut original = original.clone();
    original.completed.clone_from(&current.completed);
    if original != *current {
        return Err(DeploymentError::InputConflict);
    }
    Ok(())
}
