use run_domain::{Run, RunState};
use runtime_types::RunId;
use std::sync::Arc;
use tokio::sync::broadcast;
use vm_trait::{VmEvent, VmInstance};
use volume_trait::{VolumeAttachment, VolumeLease};

use super::{
    OrchestratorError, RunOrchestrator,
    prepare::{PreparedStart, Stage, finished},
};

pub enum ProvisionResult {
    Started(Box<StartedRun>),
    Finished(Box<Run>),
}

pub struct StartedRun {
    pub run: Run,
    pub attachment: Option<VolumeAttachment>,
    pub workspace_enabled: bool,
    pub instance: Arc<dyn VmInstance>,
    pub events: broadcast::Receiver<VmEvent>,
    pub lease: Option<VolumeLease>,
    pub execution_timeout: Option<std::time::Duration>,
}

pub async fn provision_and_start(
    orchestrator: &RunOrchestrator,
    run_id: RunId,
    prepared: PreparedStart,
) -> Result<ProvisionResult, OrchestratorError> {
    let PreparedStart {
        run,
        attachment,
        workspace_enabled,
        expected_authority_ack,
        spec,
        execution_timeout,
    } = prepared;
    let instance = match provision_vm(orchestrator, run_id, &run, attachment.as_ref(), spec).await?
    {
        Stage::Continue(instance) => instance,
        Stage::Finished(run) => return Ok(ProvisionResult::Finished(run)),
    };
    match start_vm(
        orchestrator,
        StartContext {
            run,
            attachment,
            workspace_enabled,
            expected_authority_ack,
            execution_timeout,
        },
        instance,
    )
    .await?
    {
        Stage::Continue(started) => Ok(ProvisionResult::Started(Box::new(started))),
        Stage::Finished(run) => Ok(ProvisionResult::Finished(run)),
    }
}

async fn provision_vm(
    orchestrator: &RunOrchestrator,
    run_id: RunId,
    run: &Run,
    attachment: Option<&VolumeAttachment>,
    spec: vm_trait::VmSpec,
) -> Result<Stage<Arc<dyn VmInstance>>, OrchestratorError> {
    if let Err(error) = orchestrator.launch_authorizer.authorize(run).await {
        return orchestrator
            .fail_with_resources(
                run_id,
                attachment.map(|value| &value.lease),
                None,
                &error.to_string(),
            )
            .await
            .map(finished);
    }
    if let Err(error) = orchestrator.secrets.reauthorize(run).await {
        return orchestrator
            .fail_with_resources(
                run_id,
                attachment.map(|value| &value.lease),
                None,
                &error.to_string(),
            )
            .await
            .map(finished);
    }
    if let Err(error) = orchestrator.authority.reauthorize(run).await {
        return orchestrator
            .fail_with_resources(
                run_id,
                attachment.map(|value| &value.lease),
                None,
                &error.to_string(),
            )
            .await
            .map(finished);
    }
    let instance = match orchestrator.provider.provision(spec).await {
        Ok(instance) => instance,
        Err(error) => {
            return orchestrator
                .fail_with_resources(
                    run_id,
                    attachment.map(|value| &value.lease),
                    None,
                    &error.to_string(),
                )
                .await
                .map(finished);
        }
    };
    orchestrator
        .active
        .lock()
        .await
        .insert(run_id, Arc::clone(&instance));
    if orchestrator
        .repository
        .get(run_id)
        .await?
        .cancel_requested_at
        .is_some()
    {
        orchestrator
            .repository
            .transition(run_id, RunState::Cancelled, None, None)
            .await?;
        return orchestrator
            .cleanup(run_id, attachment.map(|value| &value.lease), Some(instance))
            .await
            .map(finished);
    }
    Ok(Stage::Continue(instance))
}

struct StartContext {
    run: Run,
    attachment: Option<VolumeAttachment>,
    workspace_enabled: bool,
    expected_authority_ack: Option<(uuid::Uuid, u64)>,
    execution_timeout: Option<std::time::Duration>,
}

async fn start_vm(
    orchestrator: &RunOrchestrator,
    context: StartContext,
    instance: Arc<dyn VmInstance>,
) -> Result<Stage<StartedRun>, OrchestratorError> {
    let run_id = context.run.id;
    if let Err(error) = orchestrator
        .repository
        .transition(run_id, RunState::Starting, None, None)
        .await
    {
        orchestrator.abort_vm_keep_lease(run_id, &instance).await;
        return Err(error.into());
    }
    let events = instance.subscribe_events();
    if let Err(error) = instance.start().await {
        return handle_start_failure(
            orchestrator,
            run_id,
            context.attachment.as_ref(),
            instance,
            error,
        )
        .await;
    }
    finish_started(orchestrator, context, instance, events).await
}

async fn handle_start_failure(
    orchestrator: &RunOrchestrator,
    run_id: RunId,
    attachment: Option<&VolumeAttachment>,
    instance: Arc<dyn VmInstance>,
    error: vm_trait::VmError,
) -> Result<Stage<StartedRun>, OrchestratorError> {
    let cancelled = orchestrator
        .repository
        .get(run_id)
        .await?
        .cancel_requested_at
        .is_some();
    if cancelled {
        orchestrator
            .repository
            .transition(run_id, RunState::Cancelled, None, None)
            .await?;
        return orchestrator
            .cleanup(run_id, attachment.map(|value| &value.lease), Some(instance))
            .await
            .map(finished);
    }
    orchestrator
        .fail_with_resources(
            run_id,
            attachment.map(|value| &value.lease),
            Some(instance),
            &error.to_string(),
        )
        .await
        .map(finished)
}

async fn finish_started(
    orchestrator: &RunOrchestrator,
    context: StartContext,
    instance: Arc<dyn VmInstance>,
    mut events: tokio::sync::broadcast::Receiver<VmEvent>,
) -> Result<Stage<StartedRun>, OrchestratorError> {
    if let Some(expected) = context.expected_authority_ack {
        if let Err(error) = orchestrator
            .await_runtime_authority_acknowledgement(
                &context.run,
                &mut events,
                expected,
                orchestrator.cancellation_timeout,
            )
            .await
        {
            return orchestrator
                .fail_with_resources(
                    context.run.id,
                    context.attachment.as_ref().map(|value| &value.lease),
                    Some(instance),
                    &error.to_string(),
                )
                .await
                .map(finished);
        }
    }
    let lease = match context.attachment.as_ref() {
        Some(attachment) => match orchestrator.volumes.mark_attached(&attachment.lease).await {
            Ok(lease) => Some(lease),
            Err(error) => {
                orchestrator
                    .abort_vm_keep_lease(context.run.id, &instance)
                    .await;
                return Err(error.into());
            }
        },
        None => None,
    };
    if orchestrator
        .repository
        .get(context.run.id)
        .await?
        .cancel_requested_at
        .is_some()
    {
        orchestrator
            .repository
            .transition(context.run.id, RunState::Cancelled, None, None)
            .await?;
        return orchestrator
            .cleanup(context.run.id, lease.as_ref(), Some(instance))
            .await
            .map(finished);
    }
    if let Err(error) = orchestrator
        .repository
        .transition(context.run.id, RunState::Running, None, None)
        .await
    {
        orchestrator
            .abort_vm_keep_lease(context.run.id, &instance)
            .await;
        return Err(error.into());
    }
    Ok(Stage::Continue(StartedRun {
        run: context.run,
        attachment: context.attachment,
        workspace_enabled: context.workspace_enabled,
        instance,
        events,
        lease,
        execution_timeout: context.execution_timeout,
    }))
}
