use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use release_domain::{AgentInstanceId, AgentInstanceRevisionId};
use release_service::{
    ActivateInstance, InstanceActivationAdmission, InstanceActivationId, InstanceExecutionError,
};

use super::{
    PostgresInstanceExecutionService, reservation,
    rows::{ActivationContext, ActivationResult, database},
};

pub async fn activate(
    service: &PostgresInstanceExecutionService,
    identity: &AuthenticatedIdentity,
    command: ActivateInstance,
) -> Result<InstanceActivationAdmission, InstanceExecutionError> {
    let mut tx = begin_actor_transaction(&service.pool, identity)
        .await
        .map_err(|error| database(&error))?;
    service
        .authorization
        .require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id().as_uuid()),
        )
        .await
        .map_err(|error| authorization(&error))?;
    let context: ActivationContext =
        sqlx::query_as("SELECT * FROM qualified_instance_activation_context($1)")
            .bind(command.instance_id().as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| database(&error))?;
    service
        .authorization
        .require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, context.release_agent),
        )
        .await
        .map_err(|error| authorization(&error))?;
    // This is a freshly owned transaction. No rows have been locked; reserving
    // the actual ordered keys here cannot borrow a Project-held caller context.
    reservation::reserve(&mut tx, &context, command).await?;
    let config = serde_json::to_vec(&service.configuration.json())
        .map_err(|_| InstanceExecutionError::ConfigurationConflict)?;
    let result: ActivationResult =
        sqlx::query_as("SELECT * FROM write_qualified_instance_activation($1,$2,$3,$4,$5,$6,$7)")
            .bind(command.activation_id().as_uuid())
            .bind(command.command_key().as_bytes().as_slice())
            .bind(command.instance_id().as_uuid())
            .bind(command.expected_revision_id().as_uuid())
            .bind(
                i64::try_from(command.expected_creation_version())
                    .map_err(|_| InstanceExecutionError::InvalidVersion)?,
            )
            .bind(config)
            .bind(AUTHORIZATION_MODEL_VERSION)
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| database(&error))?;
    let admitted = InstanceActivationAdmission::new(
        InstanceActivationId::from_uuid(result.activation)?,
        AgentInstanceId::from_uuid(result.instance),
        AgentInstanceRevisionId::from_uuid(result.revision),
        u64::try_from(result.version).map_err(|_| InstanceExecutionError::OutcomeUncertain)?,
    )?;
    let revision_matches = admitted.revision_id() == command.expected_revision_id();
    if admitted.activation_id() != command.activation_id()
        || admitted.instance_id() != command.instance_id()
        || !revision_matches
    {
        return Err(InstanceExecutionError::InputConflict);
    }
    tx.commit().await.map_err(|error| database(&error))?;
    Ok(admitted)
}

const fn authorization(error: &crate::ReleaseServiceError) -> InstanceExecutionError {
    if matches!(error, crate::ReleaseServiceError::AuthorizationDenied) {
        InstanceExecutionError::AuthorizationDenied
    } else {
        InstanceExecutionError::OutcomeUncertain
    }
}
