use crate::{
    managed_backend::recipe::Failure,
    recipe_engine::{RecipeApplicationError, RecipeInstallation},
    rpc::{
        MutationReceipts, RpcError,
        project::{get_recipe_deployment::projection, opaque},
    },
};
use recipe_application::{CommandIdentity, DeploymentId, PlannedResourceIdentity};
use rpc_proto::messages::hephaestus::{
    common::v1::MutationReceipt,
    project::v1::{RecipeActivationResult, RecipeActivationResultStatus, RecipeDeployment},
};
use std::collections::BTreeSet;
use uuid::Uuid;

pub(super) fn result(
    value: &RecipeInstallation,
    command: CommandIdentity,
    project: Uuid,
    deployment: DeploymentId,
    receipts: &MutationReceipts,
) -> Result<
    (
        RecipeDeployment,
        MutationReceipt,
        Vec<RecipeActivationResult>,
    ),
    RpcError,
> {
    let progress = projection::project(&value.ledger, project, deployment)?;
    let receipt = value
        .completion
        .as_ref()
        .unwrap_or(&value.admission.receipt);
    let receipt = receipts.recipe_receipt(receipt, command, project, deployment)?;
    if value.activations.len() > recipe_domain::MAX_RECIPE_ITEMS {
        return Err(RpcError::FailedPrecondition);
    }
    let mut seen = BTreeSet::new();
    let activations = value
        .activations
        .iter()
        .map(|attempt| {
            if !seen.insert(&attempt.resource) {
                return Err(RpcError::FailedPrecondition);
            }
            let saved = value
                .ledger
                .intent
                .resources()
                .get(&attempt.resource)
                .ok_or(RpcError::FailedPrecondition)?;
            let PlannedResourceIdentity::Instance { id, revision_id } = saved.identity() else {
                return Err(RpcError::FailedPrecondition);
            };
            let (status, activation_id) = match &attempt.result {
                Ok(admitted)
                    if admitted.instance_id() == id
                        && admitted.revision_id() == revision_id
                        && !admitted.activation_id().as_uuid().is_nil() =>
                {
                    (
                        RecipeActivationResultStatus::Admitted,
                        Some(opaque(admitted.activation_id().as_uuid())),
                    )
                }
                Ok(_) => return Err(RpcError::FailedPrecondition),
                Err(_) => (RecipeActivationResultStatus::Held, None),
            };
            Ok(RecipeActivationResult {
                resource: attempt.resource.as_str().to_owned(),
                status: status.into(),
                activation_id: activation_id.into(),
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((progress, receipt, activations))
}
pub(super) const fn failure(error: &Failure) -> RpcError {
    match error {
        Failure::Execution(RecipeApplicationError::Deployment(error)) => match error {
            recipe_application::DeploymentError::AuthorizationDenied => RpcError::PermissionDenied,
            recipe_application::DeploymentError::InvalidPlanningInput
            | recipe_application::DeploymentError::InvalidIdentifier
            | recipe_application::DeploymentError::InvalidKey
            | recipe_application::DeploymentError::Recipe(_) => RpcError::InvalidArgument,
            recipe_application::DeploymentError::Unavailable
            | recipe_application::DeploymentError::Repository => RpcError::Unavailable,
            _ => RpcError::FailedPrecondition,
        },
        Failure::Join(_) | Failure::Unsettled => RpcError::Unavailable,
        _ => RpcError::FailedPrecondition,
    }
}
