//! One coherent read-only current-manager snapshot; original identities are DATA.
use super::{
    super::{PostgresPreparedInstanceSafetyRepository, query},
    loader, wire,
};
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentOperation, EffectClaim, ExistingInstanceClosureRepository,
    ExistingInstanceClosureRequest, OriginalInstanceClosure,
};
use serde_json::{Value, json};

fn claim(claim: &EffectClaim) -> Result<Value, DeploymentError> {
    Ok(
        json!({"command_id":claim.command.id(),"deployment_id":claim.deployment_id,"resource":claim.resource,
        "identity":serde_json::to_value(claim.identity).map_err(|_|DeploymentError::Serialization)?,
        "input_hash":claim.input_hash,"resource_version":claim.resource_version,"generation":claim.generation,
        "action":claim.action,"attempt_id":claim.provenance.attempt_id,"actor_id":claim.provenance.actor_id,
        "request_id":claim.provenance.request_id}),
    )
}
fn expected(closure: &OriginalInstanceClosure) -> Result<Value, DeploymentError> {
    Ok(
        json!({"drain":claim(&closure.drain)?,"actor":closure.drain.command.actor_id(),
        "idempotency":closure.drain.command.idempotency_id(),"removal_id":closure.removal_id,
        "command_key":closure.command_key,"expected_version":closure.expected_version,"creator":closure.actor,
        "request":closure.request,"input_hash":closure.input_hash}),
    )
}
pub(super) fn compare(
    data: &Value,
    closure: &OriginalInstanceClosure,
    target: &recipe_application::PreparedInstanceSafetyTarget,
) -> Result<(), DeploymentError> {
    let mut original = data.clone();
    let publication = original
        .as_object_mut()
        .ok_or(DeploymentError::ReconciliationRequired)?
        .remove("publication");
    if original != expected(closure)?
        || wire::publication(publication.as_ref(), target, &closure.drain)?
            != closure.publication_origin
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
#[async_trait]
impl ExistingInstanceClosureRepository for PostgresPreparedInstanceSafetyRepository {
    async fn load_existing_instance_closure(
        &self,
        identity: &AuthenticatedIdentity,
        request: &ExistingInstanceClosureRequest,
    ) -> Result<OriginalInstanceClosure, DeploymentError> {
        let config = self
            .remove_resume
            .as_ref()
            .ok_or(DeploymentError::InvalidAction)?;
        request
            .command()
            .validate(identity, DeploymentOperation::Remove)?;
        self.configured(request.target())?;
        let mut tx = self
            .actor
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await
            .map_err(|error| query::storage(&error))?;
        query::role(&mut tx, "hephaestus_app").await?;
        sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)")
            .bind(identity.user_id.to_string()).bind(identity.request_id.to_string()).bind(identity.idempotency_id.to_string())
            .execute(&mut *tx).await.map_err(|error|query::storage(&error))?;
        let loaded = loader::load(
            self,
            &mut tx,
            request.command(),
            request.intent(),
            &request.target().original().resource,
        )
        .await?;
        if loaded.target != *request.target() {
            return Err(DeploymentError::IntentMismatch);
        }
        loaded
            .whole
            .validate_configuration(config.profile(), config.platform())?;
        let original = request.target().original();
        let data: Value = sqlx::query_scalar(
            "SELECT load_existing_instance_closure118($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(request.command().id().as_uuid())
        .bind(request.intent().id().as_uuid())
        .bind(request.intent().project_id().as_uuid())
        .bind(request.intent().input_hash().as_bytes().as_slice())
        .bind(original.provenance.attempt_id.as_uuid())
        .bind(claim(original)?)
        .bind(
            serde_json::to_value(request.target().prepared())
                .map_err(|_| DeploymentError::Serialization)?,
        )
        .bind(config.namespace())
        .bind(config.host())
        .bind(serde_json::to_value(config.platform()).map_err(|_| DeploymentError::Serialization)?)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| query::storage(&error))?;
        compare(&data, request.expected(), &loaded.target)?;
        tx.commit().await.map_err(|error| query::storage(&error))?;
        // Returned data was independently reloaded; no status, token or ownership
        // claim is attached to the original request/actor comparison fields.
        Ok(request.expected().clone())
    }
}
