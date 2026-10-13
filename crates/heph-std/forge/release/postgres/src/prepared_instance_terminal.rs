//! Exact native143 terminal boundary; observations are not physical receipts.
mod decode;
mod origin;
mod preparation;
#[cfg(test)]
mod tests;
mod transaction;
mod wire;

use crate::prepared_instance_safety;
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentOperation, PreparedInstanceDeleteObservation,
    PreparedInstanceDeleteRequest, PreparedInstanceDetachObservation,
    PreparedInstanceDetachRequest, PreparedInstanceTerminalRepository,
};
use release_service::InstanceExecutionConfiguration;
use sqlx::PgPool;

/// Protected terminal adapter retaining the original configured actor/worker pools.
///
/// Configuration is comparison data. Physical owners must remain with the owning
/// composition; this adapter neither reconstructs them nor records Applied proof.
#[derive(Clone)]
pub struct PostgresPreparedInstanceTerminalRepository {
    actor: PgPool,
    worker: PgPool,
    configuration: InstanceExecutionConfiguration,
}
impl PostgresPreparedInstanceTerminalRepository {
    /// Retains checked whole runtime configuration and separate application/worker pools.
    #[must_use]
    pub const fn new(
        actor: PgPool,
        worker: PgPool,
        configuration: InstanceExecutionConfiguration,
    ) -> Self {
        Self {
            actor,
            worker,
            configuration,
        }
    }
}

#[async_trait]
impl PreparedInstanceTerminalRepository for PostgresPreparedInstanceTerminalRepository {
    async fn observe_detach(
        &self,
        request: &PreparedInstanceDetachRequest,
    ) -> Result<PreparedInstanceDetachObservation, DeploymentError> {
        wire::configured(
            &self.configuration,
            request.target(),
            request.configuration(),
        )?;
        let mut tx = transaction::worker(&self.worker).await?;
        let result = async {
            transaction::setup(&mut tx, "hephaestus_worker", None).await?;
            let closure = origin::worker(&mut tx, request.target(), request.closure()).await?;
            let native = wire::detach(request, &self.configuration, &closure)?;
            let value = sqlx::query_scalar("SELECT observe_prepared_instance_detach143($1,$2)")
                .bind(&native)
                .bind(wire::configuration_bytes(&self.configuration)?)
                .fetch_one(&mut *tx)
                .await
                .map_err(|error| transaction::storage(&error))?;
            let row = decode::detach(&value, &native)?;
            let safety = prepared_instance_safety::safety_observation(row, request.target())?;
            PreparedInstanceDetachObservation::new(request.clone(), safety)
        }
        .await;
        transaction::finish(tx, result, false).await
    }

    async fn delete_for_remove(
        &self,
        identity: &AuthenticatedIdentity,
        request: &PreparedInstanceDeleteRequest,
    ) -> Result<PreparedInstanceDeleteObservation, DeploymentError> {
        request
            .current_command()
            .validate(identity, DeploymentOperation::Remove)?;
        wire::configured(
            &self.configuration,
            request.target(),
            request.configuration(),
        )?;
        let mut tx = transaction::actor(&self.actor).await?;
        let result = async {
            transaction::setup(&mut tx, "hephaestus_app", Some(identity)).await?;
            // Actual current decisions, not the historical effect/publisher actor.
            prepared_instance_safety::audit_all(&mut tx, identity, request.intent()).await?;
            let closure = origin::actor(&mut tx, request.target(), request.closure()).await?;
            let native = wire::delete(request, &self.configuration, &closure)?;
            let rows = sqlx::query_as("SELECT terminal_operation_id,after_instance_version FROM delete_prepared_instance143($1,$2)")
                .bind(native).bind(wire::configuration_bytes(&self.configuration)?)
                .fetch_all(&mut *tx).await.map_err(|error| transaction::storage(&error))?;
            decode::deleted(request, &rows)
        }.await;
        transaction::finish(tx, result, true).await
    }

    async fn observe_deleted(
        &self,
        request: &PreparedInstanceDeleteRequest,
    ) -> Result<PreparedInstanceDeleteObservation, DeploymentError> {
        wire::configured(
            &self.configuration,
            request.target(),
            request.configuration(),
        )?;
        let mut tx = transaction::worker(&self.worker).await?;
        let result = async {
            transaction::setup(&mut tx, "hephaestus_worker", None).await?;
            let closure = origin::worker(&mut tx, request.target(), request.closure()).await?;
            let native = wire::delete(request, &self.configuration, &closure)?;
            let rows = sqlx::query_as("SELECT terminal_operation_id,after_instance_version FROM observe_prepared_instance_deleted143($1,$2)")
                .bind(native).bind(wire::configuration_bytes(&self.configuration)?)
                .fetch_all(&mut *tx).await.map_err(|error| transaction::storage(&error))?;
            decode::deleted(request, &rows)
        }.await;
        transaction::finish(tx, result, false).await
    }
}
