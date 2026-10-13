use super::{PostgresOwnedReadyRetentionRepository, transition};
use crate::{effect_context::Context, execution_transitions, repository_error};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{DeploymentError, PlannedResourceIdentity, RetainOwnedReady};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct ClosureRow {
    id: Uuid,
    deployment_id: Uuid,
    resource_name: String,
    attempt_id: Uuid,
    command_id: Uuid,
    operation_id: Uuid,
    fingerprint: Vec<u8>,
    vm_provider_namespace: String,
    vm_host_id: String,
}
pub async fn ensure(
    adapter: &PostgresOwnedReadyRetentionRepository,
    context: &mut Context<'_>,
    identity: &AuthenticatedIdentity,
    request: &RetainOwnedReady,
) -> Result<Uuid, DeploymentError> {
    let PlannedResourceIdentity::Volume { id, .. } = request.target.identity else {
        return Err(DeploymentError::InvalidAction);
    };
    let existing:Option<ClosureRow>=sqlx::query_as("SELECT id,deployment_id,resource_name,attempt_id,command_id,operation_id,fingerprint,vm_provider_namespace,vm_host_id FROM volume_owned_ready_retention_closures WHERE volume_id=$1")
        .bind(id.as_uuid()).fetch_optional(&mut *context.tx).await.map_err(repository_error)?;
    if let Some(row) = existing {
        if row.deployment_id != request.target.deployment_id.as_uuid()
            || row.resource_name != request.target.resource.as_str()
            || row.attempt_id != request.target.provenance.attempt_id.as_uuid()
            || row.vm_provider_namespace != adapter.namespace
            || row.vm_host_id != adapter.host
        {
            return Err(DeploymentError::InputConflict);
        }
        if row.command_id == request.command.id().as_uuid() {
            if row.operation_id != request.operation_id.as_uuid()
                || row.fingerprint.as_slice() != transition::fingerprint(request)?.as_bytes()
            {
                return Err(DeploymentError::InputConflict);
            }
        } else {
            require_fresh_remove(context, request, &row).await?;
        }
        return Ok(row.id);
    }
    super::context::current(context, request).await?;
    let closure = Uuid::new_v5(
        &request.operation_id.as_uuid(),
        b"heph-owned-ready-retention-closure-v1\0",
    );
    let (event, cursor, version) = execution_transitions::event(
        &mut context.tx,
        identity,
        closure,
        context.organization,
        context.snapshot.intent.project_id(),
    )
    .await?;
    let inserted=sqlx::query("INSERT INTO volume_owned_ready_retention_closures(id,volume_id,project_id,deployment_id,resource_name,attempt_id,command_id,operation_id,fingerprint,expected_deployment_version,expected_resource_version,seal_id,creation_operation_id,provisioning_operation_id,generation,purpose_hash,ready_observation_id,vm_provider_namespace,vm_host_id,actor_id,request_id,event_id,event_cursor,event_aggregate_version)
        SELECT $1,seal.volume_id,seal.project_id,$2,$3,$4,$5,$6,$7,$8,$9,seal.id,seal.operation_id,operation.id,operation.generation,purpose.purpose_hash,observed.id,$10,$11,$12,$13,$14,$15,$16
        FROM volume_creation_seals seal JOIN volume_owned_backing_purposes purpose ON purpose.volume_id=seal.volume_id AND purpose.seal_id=seal.id
        JOIN volume_owned_provisioning_operations operation ON operation.volume_id=seal.volume_id AND operation.expected_generation=0 AND operation.generation=1
        JOIN volume_owned_backing_observations observed ON observed.operation_id=operation.id AND observed.volume_id=seal.volume_id AND observed.generation=1 AND observed.phase='ready'
        WHERE seal.volume_id=$17 AND seal.attempt_id=$4")
        .bind(closure).bind(request.target.deployment_id.as_uuid()).bind(request.target.resource.as_str())
        .bind(request.target.provenance.attempt_id.as_uuid()).bind(request.command.id().as_uuid())
        .bind(request.operation_id.as_uuid()).bind(transition::fingerprint(request)?.as_bytes().as_slice())
        .bind(i64::try_from(request.expected_deployment_version).map_err(|_|DeploymentError::StaleClaim)?)
        .bind(i64::try_from(request.expected_resource_version).map_err(|_|DeploymentError::StaleClaim)?)
        .bind(&adapter.namespace).bind(&adapter.host).bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid())
        .bind(event).bind(cursor).bind(version).bind(id.as_uuid()).execute(&mut *context.tx).await.map_err(repository_error)?;
    if inserted.rows_affected() != 1 {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(closure)
}

async fn require_fresh_remove(
    context: &mut Context<'_>,
    request: &RetainOwnedReady,
    row: &ClosureRow,
) -> Result<(), DeploymentError> {
    // Another manager's new command requires its own exact116 admission, never
    // an old receipt-version convention or the original actor's command.
    let exact: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_owned_ready_remove_admissions WHERE command_id=$1 AND closure_id=$2 AND expected_version=$3)")
        .bind(request.command.id().as_uuid()).bind(row.id)
        .bind(i64::try_from(request.expected_deployment_version).map_err(|_|DeploymentError::StaleClaim)?)
        .fetch_one(&mut *context.tx).await.map_err(repository_error)?;
    if row.operation_id == request.operation_id.as_uuid() || !exact {
        return Err(DeploymentError::InputConflict);
    }
    Ok(())
}
