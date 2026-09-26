use super::{
    OrchestratorError, RunOrchestrator,
    prepare::{BoundStart, PreparedResources, Stage, finished},
};

fn terminal<T>(
    result: Result<run_domain::Run, OrchestratorError>,
) -> Result<Stage<T>, OrchestratorError> {
    result.map(finished)
}

pub async fn prepare_resources(
    orchestrator: &RunOrchestrator,
    run_id: runtime_types::RunId,
    bound: &BoundStart,
) -> Result<Stage<PreparedResources>, OrchestratorError> {
    let base = match prepare_base(orchestrator, run_id, bound).await? {
        Stage::Continue(base) => base,
        Stage::Finished(run) => return Ok(Stage::Finished(run)),
    };
    let BaseResources {
        workspace_enabled,
        mut mounts,
        authority,
    } = base;
    let runtime_git_workspace = match orchestrator
        .runtime_git_workspace
        .prepare_runtime_git(&bound.run)
        .await
    {
        Ok(workspace) => workspace,
        Err(error) => {
            return terminal(
                orchestrator
                    .fail_with_resources(
                        run_id,
                        bound.attachment.as_ref().map(|value| &value.lease),
                        None,
                        &error.to_string(),
                    )
                    .await,
            );
        }
    };
    if let Some(workspace) = &runtime_git_workspace {
        mounts.push(workspace.mount.clone());
    }
    if let Err(error) = orchestrator.resource_observer.record(&bound.run).await {
        return terminal(
            orchestrator
                .fail_with_resources(
                    run_id,
                    bound.attachment.as_ref().map(|value| &value.lease),
                    None,
                    &error.to_string(),
                )
                .await,
        );
    }
    Ok(Stage::Continue(PreparedResources {
        workspace_enabled,
        mounts,
        authority,
        runtime_git_workspace,
    }))
}

struct BaseResources {
    workspace_enabled: bool,
    mounts: Vec<vm_trait::VmMount>,
    authority: super::authority::PreparedRunAuthority,
}

async fn prepare_base(
    orchestrator: &RunOrchestrator,
    run_id: runtime_types::RunId,
    bound: &BoundStart,
) -> Result<Stage<BaseResources>, OrchestratorError> {
    let workspace = match orchestrator.workspaces.prepare(&bound.run).await {
        Ok(workspace) => workspace,
        Err(error) => {
            return terminal(
                orchestrator
                    .fail_with_resources(
                        run_id,
                        bound.attachment.as_ref().map(|value| &value.lease),
                        None,
                        &error.to_string(),
                    )
                    .await,
            );
        }
    };
    let workspace_enabled = workspace.id.is_some();
    let runtime = match orchestrator.runtimes.prepare(&bound.run).await {
        Ok(runtime) => runtime,
        Err(error) => {
            return terminal(
                orchestrator
                    .fail_with_resources(
                        run_id,
                        bound.attachment.as_ref().map(|value| &value.lease),
                        None,
                        &error.to_string(),
                    )
                    .await,
            );
        }
    };
    let mut mounts = workspace.mounts;
    mounts.extend(runtime.mounts);
    let secrets = match orchestrator.secrets.prepare(&bound.run).await {
        Ok(secrets) => secrets,
        Err(error) => {
            return terminal(
                orchestrator
                    .fail_with_resources(
                        run_id,
                        bound.attachment.as_ref().map(|value| &value.lease),
                        None,
                        &error.to_string(),
                    )
                    .await,
            );
        }
    };
    mounts.extend(secrets.mounts);
    let authority = match orchestrator.authority.prepare(&bound.run).await {
        Ok(authority) => authority,
        Err(error) => {
            return terminal(
                orchestrator
                    .fail_with_resources(
                        run_id,
                        bound.attachment.as_ref().map(|value| &value.lease),
                        None,
                        &error.to_string(),
                    )
                    .await,
            );
        }
    };
    Ok(Stage::Continue(BaseResources {
        workspace_enabled,
        mounts,
        authority,
    }))
}
