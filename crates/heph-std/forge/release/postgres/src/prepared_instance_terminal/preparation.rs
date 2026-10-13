//! Current-authorized initial preparation; no worker receipt or effect writes.
mod closure;
mod decode;
#[cfg(test)]
mod tests;

use super::{PostgresPreparedInstanceTerminalRepository, transaction, wire};
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentOperation, PreparedInstanceDeleteRequest,
    PreparedInstanceDetachRequest, PreparedInstanceTerminalPreparationInput,
    PreparedInstanceTerminalPreparationRepository, ResourceAction,
};

#[async_trait]
impl PreparedInstanceTerminalPreparationRepository for PostgresPreparedInstanceTerminalRepository {
    async fn prepare_detach(
        &self,
        identity: &AuthenticatedIdentity,
        input: &PreparedInstanceTerminalPreparationInput,
    ) -> Result<PreparedInstanceDetachRequest, DeploymentError> {
        checked(identity, input, ResourceAction::Detach)?;
        wire::configured(&self.configuration, input.target(), input.configuration())?;
        let mut tx = self
            .actor
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await
            .map_err(|error| transaction::storage(&error))?;
        let result = async {
            transaction::setup(&mut tx, "hephaestus_app", Some(identity)).await?;
            let (original, _) = closure::load(&mut tx, input, &self.configuration).await?;
            PreparedInstanceDetachRequest::new(
                input.intent().clone(),
                input.target().clone(),
                original,
                input.effect().clone(),
                input.current_command(),
                input.configuration().clone(),
            )
        }
        .await;
        transaction::finish(tx, result, false).await
    }

    async fn prepare_delete(
        &self,
        identity: &AuthenticatedIdentity,
        input: &PreparedInstanceTerminalPreparationInput,
    ) -> Result<PreparedInstanceDeleteRequest, DeploymentError> {
        checked(identity, input, ResourceAction::Delete)?;
        wire::configured(&self.configuration, input.target(), input.configuration())?;
        let mut tx = self
            .actor
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await
            .map_err(|error| transaction::storage(&error))?;
        let result = async {
            transaction::setup(&mut tx, "hephaestus_app", Some(identity)).await?;
            let (original, full_native) =
                closure::load(&mut tx, input, &self.configuration).await?;
            let native = decode::transport(input, &self.configuration, &full_native)?;
            // NULL exists ONLY in comparison transport. The protected native
            // reader positively selects the original live/saved before-CAS.
            let value = sqlx::query_scalar(
                "SELECT load_prepared_instance_delete_continuation143($1,$2,$3)",
            )
            .bind(input.current_command().id().as_uuid())
            .bind(&native)
            .bind(wire::configuration_bytes(&self.configuration)?)
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| transaction::storage(&error))?;
            let version = decode::continuation(&value, &native)?;
            PreparedInstanceDeleteRequest::new(
                input.intent().clone(),
                input.target().clone(),
                original,
                input.effect().clone(),
                input.current_command(),
                input.configuration().clone(),
                version,
            )
        }
        .await;
        transaction::finish(tx, result, false).await
    }
}
fn checked(
    identity: &AuthenticatedIdentity,
    input: &PreparedInstanceTerminalPreparationInput,
    action: ResourceAction,
) -> Result<(), DeploymentError> {
    input
        .current_command()
        .validate(identity, DeploymentOperation::Remove)?;
    if input.effect().action != action {
        return Err(DeploymentError::InvalidAction);
    }
    Ok(())
}
