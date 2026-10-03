//! Atomic Invocation admission only; this neither activates nor starts a VM.

use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use release_domain::{AgentInstanceId, AgentInstanceRevisionId};
use release_service::{
    InstanceExecutionError, InstanceInvocationAdmission, InstanceInvocationId, InvokeInstance,
};
use runtime_types::{CommandId, RunId};
use sqlx::{Postgres, Transaction};

use super::{
    PostgresInstanceExecutionService, invocation_reservation,
    rows::{ActivationContext, InvocationResult, database},
};

pub async fn invoke(
    service: &PostgresInstanceExecutionService,
    identity: &AuthenticatedIdentity,
    command: InvokeInstance,
) -> Result<InstanceInvocationAdmission, InstanceExecutionError> {
    let mut tx = begin_actor_transaction(&service.pool, identity)
        .await
        .map_err(|error| database(&error))?;
    let context = authorize(service, &mut tx, identity, command).await?;
    // Fresh owned transaction: ordered original107 and invocation reservations
    // precede Project/Deployment/Resource/late consumer locks in the writer.
    invocation_reservation::reserve(&mut tx, &context, command.command_key()).await?;
    let config = serde_json::to_vec(&service.configuration.json())
        .map_err(|_| InstanceExecutionError::ConfigurationConflict)?;
    let row: InvocationResult = sqlx::query_as(
        "SELECT * FROM write_qualified_instance_invocation($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(command.invocation_id().as_uuid())
    .bind(command.command_key().as_bytes().as_slice())
    .bind(command.run_id().as_uuid())
    .bind(command.start_command_id().as_uuid())
    .bind(command.instance_id().as_uuid())
    .bind(command.expected_revision_id().as_uuid())
    .bind(config)
    .bind(AUTHORIZATION_MODEL_VERSION)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| database(&error))?;
    let admission = checked_result(&row, command)?;
    // Unknown commit cannot be represented as absence or a replacement Run.
    tx.commit().await.map_err(|error| database(&error))?;
    Ok(admission)
}

async fn authorize(
    service: &PostgresInstanceExecutionService,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    command: InvokeInstance,
) -> Result<ActivationContext, InstanceExecutionError> {
    service
        .authorization
        .require(
            tx,
            identity,
            Permission::CanExecute,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id().as_uuid()),
        )
        .await
        .map_err(|error| authorization(&error))?;
    let context: ActivationContext =
        sqlx::query_as("SELECT * FROM qualified_instance_invocation_context($1)")
            .bind(command.instance_id().as_uuid())
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| database(&error))?;
    service
        .authorization
        .require(
            tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, context.release_agent),
        )
        .await
        .map_err(|error| authorization(&error))?;
    Ok(context)
}

fn checked_result(
    row: &InvocationResult,
    command: InvokeInstance,
) -> Result<InstanceInvocationAdmission, InstanceExecutionError> {
    let result = InstanceInvocationAdmission::new(
        InstanceInvocationId::from_uuid(row.invocation)?,
        AgentInstanceId::from_uuid(row.instance),
        AgentInstanceRevisionId::from_uuid(row.revision),
        RunId::from_uuid(row.run),
        CommandId::from_uuid(row.start_command),
    )?;
    let revision_matches = result.revision_id() == command.expected_revision_id();
    if result.invocation_id() != command.invocation_id()
        || result.instance_id() != command.instance_id()
        || !revision_matches
        || result.run_id() != command.run_id()
        || result.start_command_id() != command.start_command_id()
    {
        return Err(InstanceExecutionError::InputConflict);
    }
    Ok(result)
}

const fn authorization(error: &crate::ReleaseServiceError) -> InstanceExecutionError {
    if matches!(error, crate::ReleaseServiceError::AuthorizationDenied) {
        InstanceExecutionError::AuthorizationDenied
    } else {
        InstanceExecutionError::OutcomeUncertain
    }
}
