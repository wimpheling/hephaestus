use release_service::InstanceExecutionConfiguration;
use run_domain::{RunKind, StartRun};
use run_orchestrator::RepositoryError;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    errors::storage,
    model::parse_state,
    qualified_invocation_routing::{
        QualifiedInvocationEvidence, QualifiedInvocationRoute, QualifiedInvocationRoutingHoldReason,
    },
};

#[derive(sqlx::FromRow)]
pub(super) struct OriginalRun {
    pub(super) id: Uuid,
    pub(super) command_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Option<Uuid>,
    pub(super) run_kind: String,
    requires_state: bool,
    pub(super) execution_profile: String,
    vm_id: Option<String>,
    vm_provider_namespace: Option<String>,
    vm_host_id: Option<String>,
    vm_binding_kind: Option<String>,
    state: String,
    cancel_requested_at: Option<time::OffsetDateTime>,
    volume_acquisition_closed_at: Option<time::OffsetDateTime>,
}

#[derive(sqlx::FromRow)]
pub(super) struct Invocation {
    id: Uuid,
    run_id: Uuid,
    start_command_id: Uuid,
    actor_id: Uuid,
    request_id: Uuid,
    command_key: Vec<u8>,
    input_hash: Vec<u8>,
    config_bytes: Vec<u8>,
    config_hash: Vec<u8>,
    config_json: Value,
    admission_version: i32,
}

#[derive(sqlx::FromRow)]
pub(super) struct Inbox {
    command_type: String,
    payload: Value,
    pub(super) processed: bool,
}

pub(super) fn compare(
    run: &OriginalRun,
    receipt: &Invocation,
    inbox: &Inbox,
    expected: &InstanceExecutionConfiguration,
    supplied: Option<&StartRun>,
) -> Result<Result<QualifiedInvocationRoute, QualifiedInvocationRoutingHoldReason>, RepositoryError>
{
    use QualifiedInvocationRoutingHoldReason as Hold;
    if [
        run.id,
        run.command_id,
        run.instance_id,
        run.instance_revision_id,
        run.release_id,
        run.release_agent_id,
        receipt.id,
        receipt.actor_id,
        receipt.request_id,
    ]
    .iter()
    .any(Uuid::is_nil)
        || receipt.run_id != run.id
        || receipt.start_command_id != run.command_id
        || receipt.admission_version != 115
        || run.attachment_id.is_some()
        || run.vm_binding_kind.as_deref() != Some("planned")
        || run.vm_id.as_deref() != Some(run.id.to_string().as_str())
    {
        return Ok(Err(Hold::ContradictoryOriginal));
    }
    if run.vm_provider_namespace.as_deref() != Some(expected.provider_scope().namespace())
        || run.vm_host_id.as_deref() != Some(expected.provider_scope().host_id())
    {
        return Ok(Err(Hold::ForeignOwner));
    }
    if !configuration_matches(receipt, expected)? {
        return Ok(Err(Hold::ConfigurationMismatch));
    }
    let Ok(command_key) = receipt.command_key.as_slice().try_into() else {
        return Ok(Err(Hold::ContradictoryOriginal));
    };
    let Ok(input_hash) = receipt.input_hash.as_slice().try_into() else {
        return Ok(Err(Hold::ContradictoryOriginal));
    };
    let start = StartRun {
        command_id: run.command_id.into(),
        run_id: run.id.into(),
        instance_id: run.instance_id.into(),
        instance_revision_id: run.instance_revision_id.into(),
        release_id: run.release_id.into(),
        release_agent_id: run.release_agent_id.into(),
        attachment_id: None,
        kind: RunKind::Invocation,
        requires_state: run.requires_state,
    };
    if inbox.command_type != "start_run"
        || inbox.payload != serde_json::to_value(&start).map_err(storage)?
        || supplied.is_some_and(|command| command != &start)
    {
        return Ok(Err(Hold::ContradictoryOriginal));
    }
    let Ok(run_state) = parse_state(&run.state) else {
        return Ok(Err(Hold::ContradictoryOriginal));
    };
    Ok(Ok(QualifiedInvocationRoute {
        original_start: start,
        invocation_id: receipt.id,
        original_actor: receipt.actor_id,
        original_request: receipt.request_id,
        command_key,
        input_hash,
        configuration_bytes: receipt.config_bytes.clone(),
        owner: expected.provider_scope().clone(),
        evidence: QualifiedInvocationEvidence::QueuedIntent115,
        run_state,
        cancellation_requested: run.cancel_requested_at.is_some(),
        acquisition_closed: run.volume_acquisition_closed_at.is_some(),
    }))
}

fn configuration_matches(
    receipt: &Invocation,
    expected: &InstanceExecutionConfiguration,
) -> Result<bool, RepositoryError> {
    let expected_json = expected.canonical_json();
    let expected_bytes = serde_json::to_vec(&expected_json).map_err(storage)?;
    let actual_hash: [u8; 32] = Sha256::digest(&receipt.config_bytes).into();
    Ok(receipt.config_bytes == expected_bytes
        && receipt.config_json == expected_json
        && receipt.config_hash.as_slice() == actual_hash.as_slice())
}
