use crate::rpc::{RpcError, project::opaque};
use recipe_application::{
    DeploymentId, DeploymentSnapshot, PlannedResourceIdentity, ResourceProgress,
};
use rpc_proto::messages::hephaestus::project::v1::{
    RecipeActivationObservation, RecipeDeployment, RecipeDeploymentResource,
    RecipeProgressDiagnostic, RecipeResourceKind,
};
use uuid::Uuid;

pub(in crate::rpc::project) fn project(
    snapshot: &DeploymentSnapshot,
    expected_project: Uuid,
    expected_deployment: DeploymentId,
) -> Result<RecipeDeployment, RpcError> {
    if snapshot.intent.project_id().as_uuid() != expected_project {
        return Err(RpcError::PermissionDenied);
    }
    if snapshot.intent.id() != expected_deployment
        || snapshot.resources.len() > recipe_domain::MAX_RECIPE_ITEMS
        || !snapshot
            .intent
            .resources()
            .keys()
            .eq(snapshot.resources.keys())
    {
        return Err(RpcError::FailedPrecondition);
    }
    let resources = snapshot
        .intent
        .resources()
        .iter()
        .map(|(name, planned)| {
            let progress = snapshot
                .resources
                .get(name)
                .ok_or(RpcError::FailedPrecondition)?;
            let (kind, id, revision) = identity(planned.identity())?;
            Ok(RecipeDeploymentResource {
                name: name.as_str().to_owned(),
                kind: kind.into(),
                resource_id: opaque(id).into(),
                revision_id: revision.map(opaque).into(),
                ownership: ownership(planned.ownership()).into(),
                install: install(progress.install).into(),
                removal: removal(progress.removal).into(),
                version: progress.version,
                active_attempt_id: progress
                    .active_attempt
                    .map(|id| opaque(id.as_uuid()))
                    .into(),
                diagnostic: progress
                    .diagnostic
                    .map_or(RecipeProgressDiagnostic::Unspecified, diagnostic)
                    .into(),
                recovery_required: requires_recovery(progress),
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    Ok(RecipeDeployment {
        id: opaque(snapshot.intent.id().as_uuid()).into(),
        project_id: opaque(expected_project).into(),
        key: snapshot.intent.key().as_str().to_owned(),
        version: snapshot.version,
        lifecycle: lifecycle(snapshot.lifecycle).into(),
        resources,
        activation: RecipeActivationObservation::Unobserved.into(),
        ..Default::default()
    })
}

fn identity(
    value: PlannedResourceIdentity,
) -> Result<(RecipeResourceKind, Uuid, Option<Uuid>), RpcError> {
    let (kind, id, revision) = match value {
        PlannedResourceIdentity::Volume { id, .. } => {
            (RecipeResourceKind::Volume, id.as_uuid(), None)
        }
        PlannedResourceIdentity::Instance { id, revision_id } => (
            RecipeResourceKind::Instance,
            id.as_uuid(),
            Some(revision_id.as_uuid()),
        ),
    };
    if id.is_nil() || revision.is_some_and(|value| value.is_nil()) {
        return Err(RpcError::FailedPrecondition);
    }
    Ok((kind, id, revision))
}

fn requires_recovery(progress: &ResourceProgress) -> bool {
    progress.install == recipe_application::InstallProgress::RecoveryRequired
        || progress.removal == recipe_application::RemovalProgress::RecoveryRequired
}

const fn lifecycle(
    value: recipe_application::DeploymentLifecycle,
) -> rpc_proto::messages::hephaestus::project::v1::RecipeDeploymentLifecycle {
    use recipe_application::DeploymentLifecycle;
    use rpc_proto::messages::hephaestus::project::v1::RecipeDeploymentLifecycle;
    match value {
        DeploymentLifecycle::Installing => RecipeDeploymentLifecycle::Installing,
        DeploymentLifecycle::Installed => RecipeDeploymentLifecycle::Installed,
        DeploymentLifecycle::Removing => RecipeDeploymentLifecycle::Removing,
        DeploymentLifecycle::Removed => RecipeDeploymentLifecycle::Removed,
        DeploymentLifecycle::RecoveryRequired => RecipeDeploymentLifecycle::RecoveryRequired,
    }
}

const fn ownership(
    value: recipe_application::ResourceOwnership,
) -> rpc_proto::messages::hephaestus::project::v1::RecipeResourceOwnership {
    use recipe_application::ResourceOwnership;
    use rpc_proto::messages::hephaestus::project::v1::RecipeResourceOwnership;
    match value {
        ResourceOwnership::Owned => RecipeResourceOwnership::Owned,
        ResourceOwnership::External => RecipeResourceOwnership::External,
    }
}

const fn install(
    value: recipe_application::InstallProgress,
) -> rpc_proto::messages::hephaestus::project::v1::RecipeInstallProgress {
    use recipe_application::InstallProgress;
    use rpc_proto::messages::hephaestus::project::v1::RecipeInstallProgress;
    match value {
        InstallProgress::Pending => RecipeInstallProgress::Pending,
        InstallProgress::Creating => RecipeInstallProgress::Creating,
        InstallProgress::Ready => RecipeInstallProgress::Ready,
        InstallProgress::Failed => RecipeInstallProgress::Failed,
        InstallProgress::RecoveryRequired => RecipeInstallProgress::RecoveryRequired,
    }
}

const fn removal(
    value: recipe_application::RemovalProgress,
) -> rpc_proto::messages::hephaestus::project::v1::RecipeRemovalProgress {
    use recipe_application::RemovalProgress;
    use rpc_proto::messages::hephaestus::project::v1::RecipeRemovalProgress;
    match value {
        RemovalProgress::Pending => RecipeRemovalProgress::Pending,
        RemovalProgress::Draining => RecipeRemovalProgress::Draining,
        RemovalProgress::Detached => RecipeRemovalProgress::Detached,
        RemovalProgress::Retained => RecipeRemovalProgress::Retained,
        RemovalProgress::Deleted => RecipeRemovalProgress::Deleted,
        RemovalProgress::RecoveryRequired => RecipeRemovalProgress::RecoveryRequired,
    }
}

const fn diagnostic(
    value: recipe_application::DiagnosticCode,
) -> rpc_proto::messages::hephaestus::project::v1::RecipeProgressDiagnostic {
    use recipe_application::DiagnosticCode;
    use rpc_proto::messages::hephaestus::project::v1::RecipeProgressDiagnostic;
    match value {
        DiagnosticCode::ProviderFailure => RecipeProgressDiagnostic::ProviderFailure,
        DiagnosticCode::ProviderOutcomeUnknown => RecipeProgressDiagnostic::ProviderOutcomeUnknown,
        DiagnosticCode::DependencyUnavailable => RecipeProgressDiagnostic::DependencyUnavailable,
        DiagnosticCode::AuthorizationChanged => RecipeProgressDiagnostic::AuthorizationChanged,
        DiagnosticCode::DrainPending => RecipeProgressDiagnostic::DrainPending,
        DiagnosticCode::FencingUnproven => RecipeProgressDiagnostic::FencingUnproven,
        DiagnosticCode::IntentConflict => RecipeProgressDiagnostic::IntentConflict,
    }
}
