use super::ReadyRetentionError;
use heph_runtime::VmProvider;
use recipe_application::{
    DeploymentOperation, EffectClaim, OwnedReadyRetentionPlan, PlannedResourceIdentity,
    ResourceAction,
};
use volume_trait::{
    OwnedProvisioningClaim, OwnedProvisioningExpectation, OwnedReadyRetentionContext,
};

pub fn scope(provider: &dyn VmProvider) -> Result<(String, String), ReadyRetentionError> {
    let scope = provider
        .owner_scope()
        .map_err(|_| ReadyRetentionError::Scope)?;
    Ok((scope.namespace().to_owned(), scope.host_id().to_owned()))
}
pub fn plan(
    plan: &OwnedReadyRetentionPlan,
    claim: &OwnedProvisioningClaim,
    target: &EffectClaim,
    scope: &(String, String),
) -> Result<(), ReadyRetentionError> {
    let receipt = claim.purpose.receipt();
    let registration = receipt.intent.registration();
    let fields = receipt.intent.creation().fields();
    if target.command.operation() != DeploymentOperation::Install
        || target.action != ResourceAction::Create
        || target.identity
            != (PlannedResourceIdentity::Volume {
                id: registration.id(),
                filesystem_uuid: Some(registration.filesystem_uuid()),
            })
        || fields.command_id != target.command.id().as_uuid()
        || fields.attempt_id != target.provenance.attempt_id.as_uuid()
        || fields.actor_id != target.provenance.actor_id.as_uuid()
        || fields.request_id != target.provenance.request_id.as_uuid()
        || fields.resource_version != target.resource_version
        || fields.generation != target.generation
        || fields.input_hash != *target.input_hash.as_bytes()
    {
        return Err(ReadyRetentionError::Correlation);
    }
    if plan.closure_id.is_nil()
        || plan.vm_provider_namespace != scope.0
        || plan.vm_host_id != scope.1
    {
        return Err(ReadyRetentionError::Scope);
    }
    let first_operation = OwnedProvisioningExpectation::new(
        receipt.intent.clone(),
        claim.purpose.host().to_owned(),
        claim.purpose.root().to_path_buf(),
        claim.purpose.owner_namespace(),
    )?
    .first_operation_id();
    if plan.preparation.mapping_version != 1
        || plan.preparation.creation != *receipt.intent.creation()
        || plan.preparation.volume_id != registration.id()
        || plan.preparation.project_id != registration.project_id()
        || plan.preparation.capacity_bytes != registration.capacity_bytes()
        || plan.preparation.filesystem_uuid != registration.filesystem_uuid()
        || claim.operation_id != first_operation
        || claim.generation != 1
    {
        return Err(ReadyRetentionError::Correlation);
    }
    Ok(())
}
pub fn context(
    plan: &OwnedReadyRetentionPlan,
    claim: &OwnedProvisioningClaim,
    context: &OwnedReadyRetentionContext,
    scope: &(String, String),
) -> Result<(), ReadyRetentionError> {
    context.validate()?;
    let fields = context.closure.fields();
    if context.original.claim != *claim
        || fields.id != plan.closure_id
        || fields.vm_provider_namespace != scope.0
        || fields.vm_host_id != scope.1
    {
        return Err(ReadyRetentionError::Correlation);
    }
    Ok(())
}
