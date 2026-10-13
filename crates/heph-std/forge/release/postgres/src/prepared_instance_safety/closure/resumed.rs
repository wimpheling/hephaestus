//! Current authenticated publisher, immutable FIRST Pending endpoint, exact tail.
use super::super::{PostgresPreparedInstanceSafetyRepository, query};
use super::{loader, reservations, wire, writing};
use crate::release_instance_removal_work;
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentOperation, PreparedInstanceClosureAdmission,
    ResumedPreparedInstanceClosureRequest,
};
use release_service::RequestInstanceRemoval;
use serde_json::Value;
use sqlx::{Postgres, Transaction};

pub async fn close(
    adapter: &PostgresPreparedInstanceSafetyRepository,
    identity: &AuthenticatedIdentity,
    request: &ResumedPreparedInstanceClosureRequest,
) -> Result<PreparedInstanceClosureAdmission, DeploymentError> {
    request
        .current_command()
        .validate(identity, DeploymentOperation::Remove)?;
    adapter.configured(request.target())?;
    let config = adapter
        .remove_resume
        .as_ref()
        .ok_or(DeploymentError::InvalidAction)?;
    let mut tx = begin_actor_transaction(&adapter.actor, identity)
        .await
        .map_err(|error| query::storage(&error))?;
    let result = publish(adapter, &mut tx, identity, request, config).await;
    match result {
        Ok(admission) => {
            // Native142's deferred origin/current-permission checks run here.
            tx.commit().await.map_err(|error| query::storage(&error))?;
            Ok(admission)
        }
        Err(error) => {
            // Preserve the qualification/write error even if rollback also fails.
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

async fn publish(
    adapter: &PostgresPreparedInstanceSafetyRepository,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    request: &ResumedPreparedInstanceClosureRequest,
    config: &recipe_application::RemoveResumeConfiguration,
) -> Result<PreparedInstanceClosureAdmission, DeploymentError> {
    query::role(tx, "hephaestus_app").await?;
    let loaded = loader::load(
        adapter,
        tx,
        request.current_command(),
        request.intent(),
        &request.drain().resource,
    )
    .await?;
    if loaded.target != *request.target() {
        return Err(DeploymentError::IntentMismatch);
    }
    loaded
        .whole
        .validate_configuration(config.profile(), config.platform())?;
    // Complete sorted original import keys + old Drain key, THEN all instance
    // keys. The loader above is read-only; no parent/participant lock exists yet.
    reservations::reserve(tx, &loaded.whole, request.command_key()).await?;
    let (version, context): (i64, Value) = sqlx::query_as("SELECT expected_version,context_json FROM recipe_remove_resume_admissions WHERE command_id=$1 AND deployment_id=$2 AND project_id=$3 AND intent_hash=$4 AND preparation_hash=$5 AND vm_provider_namespace=$6 AND vm_host_id=$7 AND platform_json=$8")
        .bind(request.current_command().id().as_uuid()).bind(request.intent().id().as_uuid())
        .bind(request.intent().project_id().as_uuid()).bind(request.intent().input_hash().as_bytes().as_slice())
        .bind(loaded.whole.input_hash()?.as_bytes().as_slice()).bind(config.namespace()).bind(config.host())
        .bind(serde_json::to_value(config.platform()).map_err(|_| DeploymentError::Serialization)?)
        .fetch_one(&mut **tx).await.map_err(|error| query::storage(&error))?;
    let pending = wire::admitted_pending(&context, version, request)?;
    let command = RequestInstanceRemoval {
        command_key: request.command_key(),
        removal_id: request.removal_id(),
        instance_id: request.target().prepared().instance_id(),
        expected_version: request.expected_instance_version(),
    };
    let hash = release_instance_removal_work::input_hash(identity, &command)
        .map_err(|error| writing::storage(&error))?;
    let replayed: bool = sqlx::query_scalar("SELECT prepare_resumed_prepared_instance_closure142($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)")
        .bind(request.current_command().id().as_uuid())
        .bind(request.target().original().provenance.attempt_id.as_uuid())
        .bind(writing::claim(request.target().original())?)
        .bind(serde_json::to_value(request.target().prepared()).map_err(|_| DeploymentError::Serialization)?)
        .bind(loaded.whole.canonical_bytes()?).bind(writing::claim(request.drain())?).bind(pending)
        .bind(config.namespace()).bind(config.host())
        .bind(serde_json::to_value(config.platform()).map_err(|_| DeploymentError::Serialization)?)
        .bind(version).bind(request.command_key().as_bytes().as_slice()).bind(request.removal_id().as_uuid())
        .bind(i64::try_from(request.expected_instance_version()).map_err(|_| DeploymentError::IntentMismatch)?)
        .bind(hash.as_slice()).fetch_one(&mut **tx).await.map_err(|error| query::storage(&error))?;
    // A positive winner was independently qualified by native142 before live CAS
    // and Pending-only absence checks. Never INSERT again or guess its stored CAS.
    if !replayed {
        writing::audit_all(tx, identity, request.intent()).await?;
        release_instance_removal_work::admit_resumed_request(
            tx,
            identity,
            &command,
            &hash,
            request.current_command().id(),
        )
        .await
        .map_err(|error| writing::storage(&error))?;
    }
    Ok(PreparedInstanceClosureAdmission {
        target: loaded.target,
        removal_id: request.removal_id(),
        replayed,
    })
}
