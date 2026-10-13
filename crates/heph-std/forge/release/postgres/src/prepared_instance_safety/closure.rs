//! Qualified closure schedules durable cancellation; no VM or transport IO.
mod existing;
mod loader;
mod reservations;
mod resumed;
#[cfg(test)]
mod tests;
mod wire;
mod writing;

pub use wire::publication;
pub use writing::{audit_all, claim};

use super::{PostgresPreparedInstanceSafetyRepository, query};
use async_trait::async_trait;
use authz_postgres::begin_actor_transaction;
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentIntent, DeploymentOperation,
    PreparedInstanceClosureAdmission, PreparedInstanceClosureRepository,
    PreparedInstanceClosureRequest, PreparedInstanceSafetyTarget,
};

pub async fn original(
    adapter: &PostgresPreparedInstanceSafetyRepository,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    intent: &DeploymentIntent,
    resource: &CapabilitySlotKey,
) -> Result<PreparedInstanceSafetyTarget, DeploymentError> {
    command.validate(identity, DeploymentOperation::Remove)?;
    let mut tx = adapter
        .actor
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(|error| query::storage(&error))?;
    query::role(&mut tx, "hephaestus_app").await?;
    sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)")
        .bind(identity.user_id.to_string()).bind(identity.request_id.to_string()).bind(identity.idempotency_id.to_string())
        .execute(&mut *tx).await.map_err(|error| query::storage(&error))?;
    let loaded = loader::load(adapter, &mut tx, command, intent, resource).await?;
    tx.commit().await.map_err(|error| query::storage(&error))?;
    Ok(loaded.target)
}
#[async_trait]
impl PreparedInstanceClosureRepository for PostgresPreparedInstanceSafetyRepository {
    async fn close_resumed_for_remove(
        &self,
        identity: &AuthenticatedIdentity,
        request: &recipe_application::ResumedPreparedInstanceClosureRequest,
    ) -> Result<PreparedInstanceClosureAdmission, DeploymentError> {
        resumed::close(self, identity, request).await
    }

    async fn close_for_remove(
        &self,
        identity: &AuthenticatedIdentity,
        request: &PreparedInstanceClosureRequest,
    ) -> Result<PreparedInstanceClosureAdmission, DeploymentError> {
        let command = request.drain().command;
        command.validate(identity, DeploymentOperation::Remove)?;
        self.configured(request.target())?;
        let mut tx = begin_actor_transaction(&self.actor, identity)
            .await
            .map_err(|error| query::storage(&error))?;
        query::role(&mut tx, "hephaestus_app").await?;
        let loaded = loader::load(
            self,
            &mut tx,
            command,
            request.intent(),
            &request.drain().resource,
        )
        .await?;
        if loaded.target != *request.target() {
            return Err(DeploymentError::IntentMismatch);
        }
        reservations::reserve(&mut tx, &loaded.whole, request.command_key()).await?;
        let replayed = writing::qualify(&mut tx, identity, request, &loaded.whole).await?;
        if !replayed {
            writing::write(&mut tx, identity, request).await?;
        }
        tx.commit().await.map_err(|error| query::storage(&error))?;
        Ok(PreparedInstanceClosureAdmission {
            target: loaded.target,
            removal_id: request.removal_id(),
            replayed,
        })
    }
}
