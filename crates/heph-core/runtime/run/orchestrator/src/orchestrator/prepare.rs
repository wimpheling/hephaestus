use run_domain::{Run, RunState, StartRun};
use std::time::Duration;
use uuid::Uuid;
use vm_trait::{VmError, VmId, VmSpec};
use volume_trait::VolumeAttachment;
use workspace_domain::PreparedRuntimeGitWorkspace;

use super::{
    OrchestratorError, RunOrchestrator, authority::PreparedRunAuthority,
    plural_evidence::PreparedRunVolumes, resources,
};

pub enum Stage<T> {
    Continue(T),
    Finished(Box<Run>),
}

pub enum PrepareStart {
    Continue(Box<PreparedStart>),
    Finished(Box<Run>),
}

pub struct PreparedStart {
    pub run: Run,
    pub attachment: Option<VolumeAttachment>,
    pub plural: Option<PreparedRunVolumes>,
    pub workspace_enabled: bool,
    pub expected_authority_ack: Option<(Uuid, u64)>,
    pub spec: VmSpec,
    pub execution_timeout: Option<Duration>,
}

pub struct BoundStart {
    pub run: Run,
    pub attachment: Option<VolumeAttachment>,
    pub plural: Option<PreparedRunVolumes>,
}

pub struct PreparedResources {
    pub workspace_enabled: bool,
    pub mounts: Vec<vm_trait::VmMount>,
    pub authority: PreparedRunAuthority,
    pub runtime_git_workspace: Option<PreparedRuntimeGitWorkspace>,
}

pub fn finished<T>(run: Run) -> Stage<T> {
    Stage::Finished(Box::new(run))
}

fn terminal<T>(result: Result<Run, OrchestratorError>) -> Result<Stage<T>, OrchestratorError> {
    result.map(finished)
}

pub async fn prepare_start(
    orchestrator: &RunOrchestrator,
    command: &StartRun,
) -> Result<PrepareStart, OrchestratorError> {
    let plural = if orchestrator.plural_volumes {
        let run = orchestrator.repository.get(command.run_id).await?;
        Some(orchestrator.acquire_run_volumes(&run).await?)
    } else {
        None
    };
    let attachment = match if plural.is_some() {
        Ok(Stage::Continue(
            plural
                .as_ref()
                .and_then(PreparedRunVolumes::legacy_attachment),
        ))
    } else {
        acquire_attachment(orchestrator, command).await
    }? {
        Stage::Continue(attachment) => attachment,
        Stage::Finished(run) => return Ok(PrepareStart::Finished(run)),
    };
    let bound = match bind_start(orchestrator, command, attachment, plural).await? {
        Stage::Continue(bound) => bound,
        Stage::Finished(run) => return Ok(PrepareStart::Finished(run)),
    };
    let resources = match resources::prepare_resources(orchestrator, command.run_id, &bound).await?
    {
        Stage::Continue(resources) => resources,
        Stage::Finished(run) => return Ok(PrepareStart::Finished(run)),
    };
    assemble_start(orchestrator, command.run_id, bound, resources)
        .await
        .map(|stage| match stage {
            Stage::Continue(prepared) => PrepareStart::Continue(Box::new(prepared)),
            Stage::Finished(run) => PrepareStart::Finished(run),
        })
}

async fn acquire_attachment(
    orchestrator: &RunOrchestrator,
    command: &StartRun,
) -> Result<Stage<Option<VolumeAttachment>>, OrchestratorError> {
    if !command.requires_state {
        return Ok(Stage::Continue(None));
    }
    let volume = match orchestrator
        .volumes
        .resolve_instance_state(
            command.instance_id,
            orchestrator.instance_state_capacity_bytes,
        )
        .await
    {
        Ok(volume) => volume,
        Err(error) => {
            return terminal(
                orchestrator
                    .fail_without_lease(command.run_id, &error.to_string())
                    .await,
            );
        }
    };
    match orchestrator
        .volumes
        .acquire(volume.id, command.run_id)
        .await
    {
        Ok(attachment) => Ok(Stage::Continue(Some(attachment))),
        Err(error) => terminal(
            orchestrator
                .fail_without_lease(command.run_id, &error.to_string())
                .await,
        ),
    }
}

async fn bind_start(
    orchestrator: &RunOrchestrator,
    command: &StartRun,
    attachment: Option<VolumeAttachment>,
    plural: Option<PreparedRunVolumes>,
) -> Result<Stage<BoundStart>, OrchestratorError> {
    let vm_id = if orchestrator.legacy_scope.is_some() {
        let run = orchestrator.repository.get(command.run_id).await?;
        orchestrator.check_legacy_open(&run).await?.vm_id().clone()
    } else if orchestrator.canonical_cleanup.is_some() {
        let persisted = orchestrator.repository.get(command.run_id).await?;
        VmId(persisted.vm_id.ok_or(crate::RepositoryError::InvalidData(
            "new run lacks its durable planned VM binding",
        ))?)
    } else {
        VmId(command.run_id.to_string())
    };
    let run = orchestrator
        .repository
        .bind_resources(
            command.run_id,
            attachment.as_ref().map(|value| value.volume.id),
            attachment.as_ref().map(|value| value.lease.id),
            attachment.as_ref().map(|value| value.lease.fencing_token),
            &vm_id.0,
        )
        .await?;
    if run.cancel_requested_at.is_some() {
        return orchestrator
            .cancel_before_vm(run, attachment.as_ref().map(|value| &value.lease))
            .await
            .map(finished);
    }
    orchestrator
        .repository
        .transition(command.run_id, RunState::Provisioning, None, None)
        .await?;
    let authorization = if let Some(volumes) = &plural {
        orchestrator
            .check_volume_authority(&run, &volumes.selections)
            .await
    } else {
        orchestrator
            .launch_authorizer
            .authorize(&run)
            .await
            .map_err(|error| {
                super::authority::RunAuthorityError::redacted(error.to_string()).into()
            })
    };
    if let Err(error) = authorization {
        return terminal(
            orchestrator
                .fail_with_resources(
                    command.run_id,
                    attachment.as_ref().map(|value| &value.lease),
                    None,
                    &error.to_string(),
                )
                .await,
        );
    }
    if run.kind != run_domain::RunKind::Invocation
        && let Err(error) = orchestrator
            .repository
            .ensure_runtime_git_provenance(&run)
            .await
    {
        return terminal(
            orchestrator
                .fail_with_resources(
                    command.run_id,
                    attachment.as_ref().map(|value| &value.lease),
                    None,
                    &error.to_string(),
                )
                .await,
        );
    }
    Ok(Stage::Continue(BoundStart {
        run,
        attachment,
        plural,
    }))
}

async fn assemble_start(
    orchestrator: &RunOrchestrator,
    run_id: runtime_types::RunId,
    bound: BoundStart,
    resources: PreparedResources,
) -> Result<Stage<PreparedStart>, OrchestratorError> {
    let PreparedResources {
        workspace_enabled,
        mut mounts,
        authority,
        runtime_git_workspace,
    } = resources;
    let built = if let Some(volumes) = &bound.plural {
        orchestrator
            .build_plural_spec(&bound.run, volumes, std::mem::take(&mut mounts))
            .await
    } else {
        orchestrator
            .build_spec(
                &bound.run,
                bound.attachment.as_ref(),
                std::mem::take(&mut mounts),
            )
            .await
    };
    let mut spec = match built {
        Ok(spec) => spec,
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
    if bound.run.kind == run_domain::RunKind::Invocation
        && (spec.runtime_git_bridge.is_some() || spec.command.working_dir.is_some())
    {
        return Err(VmError::InvalidState(
            "Invocation workload contains Git or working-directory overrides",
        )
        .into());
    }
    let expected_authority_ack = authority
        .bootstrap
        .as_ref()
        .map(|bootstrap| (bootstrap.session_id(), bootstrap.generation()));
    spec.runtime_authority = authority.bootstrap;
    if let Some(workspace) = runtime_git_workspace {
        spec.runtime_git_bridge = Some(workspace.bridge);
        spec.command.working_dir = Some(workspace_domain::RUNTIME_GIT_GUEST_PATH.into());
    }
    let execution_timeout = spec
        .labels
        .get("hephaestus.wall-clock-timeout-seconds")
        .map(|seconds| {
            seconds
                .parse::<u64>()
                .ok()
                .filter(|seconds| *seconds > 0)
                .map(Duration::from_secs)
                .ok_or_else(|| VmError::InvalidSpec {
                    field: String::from("labels.hephaestus.wall-clock-timeout-seconds"),
                    reason: String::from("must be a positive integer"),
                })
        })
        .transpose()?;
    Ok(Stage::Continue(PreparedStart {
        run: bound.run,
        attachment: bound.attachment,
        plural: bound.plural,
        workspace_enabled,
        expected_authority_ack,
        spec,
        execution_timeout,
    }))
}
