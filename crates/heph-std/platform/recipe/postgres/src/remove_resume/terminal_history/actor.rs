//! Actual current actor RRRO; saved publisher is comparison metadata only.
use super::super::decoding;
use super::{Reader, closure, encoding, original, role, storage, worker};
use async_trait::async_trait;
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentIntent, DeploymentOperation,
    PreparedInstanceDeleteCompletedHistory, PreparedInstanceDeleteHistory,
    PreparedInstanceDeleteRequest, PreparedInstanceTerminalHistoryRepository,
};
use serde_json::{Value, json};

#[async_trait]
impl PreparedInstanceTerminalHistoryRepository for Reader {
    async fn load_delete_continuation(
        &self,
        identity: &AuthenticatedIdentity,
        request: &PreparedInstanceDeleteRequest,
    ) -> Result<PreparedInstanceDeleteHistory, DeploymentError> {
        request
            .current_command()
            .validate(identity, DeploymentOperation::Remove)?;
        let mut tx = self
            .actor
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await
            .map_err(storage)?;
        let result=async {
            role(&mut tx,"hephaestus_app").await?;
            sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)")
                .bind(identity.user_id.to_string()).bind(identity.request_id.to_string()).bind(identity.idempotency_id.to_string())
                .execute(&mut *tx).await.map_err(storage)?;
            if request.configuration()!=&self.remove { return Err(DeploymentError::IntentMismatch); }
            let stored=crate::execution_preparation::hydration::load(&mut tx,request.intent()).await?
                .ok_or(DeploymentError::ReconciliationRequired)?;
            let roots=decoding::Roots {intent:request.intent(),preparation:&stored.preparation,config:&self.remove};
            let original=decoding::load_terminal_target(&mut tx,request.current_command(),&roots,&request.effect().resource).await?;
            if original!=*request.target(){return Err(DeploymentError::IntentMismatch);}
            let data:Value=sqlx::query_scalar("SELECT remove_resume_context118($1)")
                .bind(request.intent().id().as_uuid()).fetch_one(&mut *tx).await.map_err(storage)?;
            let decoded=decoding::load(&mut tx,&data,&roots).await?;
            if !decoded.closures.contains(request.closure()){return Err(DeploymentError::IntentMismatch);}
            let matching:Vec<_>=data["closures"].as_array().ok_or(DeploymentError::ReconciliationRequired)?.iter()
                .filter(|item| item["resource"]==request.effect().resource.as_str()).collect();
            if matching.len()!=1{return Err(DeploymentError::ReconciliationRequired);}
            closure::compare(&mut tx,matching[0],request).await?;
            let expected=encoding::request(self,request,matching[0])?;
            let actual:Value=sqlx::query_scalar("SELECT load_prepared_instance_delete_continuation143($1,$2,$3)")
                .bind(request.current_command().id().as_uuid()).bind(&expected).bind(encoding::configuration(self)?)
                .fetch_one(&mut *tx).await.map_err(storage)?;
            if actual.as_object().is_none_or(|object|object.len()!=3)
                || actual["version"]!=json!(1) || actual["request"]!=expected || actual.get("history").is_none(){
                return Err(DeploymentError::ReconciliationRequired);
            }
            let history=worker::decode_history(&mut tx,request,&actual["history"]).await?;
            if history.state()==recipe_application::PreparedInstanceDeleteHistoryState::Applied {
                let observed=super::applied::observe(&mut tx,request).await?;
                if history.observed_progress()!=&observed {return Err(DeploymentError::ReconciliationRequired);}
            }
            Ok(history)
        }.await;
        let rollback = tx.rollback().await.map_err(storage);
        match result {
            Err(primary) => Err(primary),
            Ok(value) => rollback.map(|()| value),
        }
    }
    async fn load_original_delete_history(
        &self,
        identity: &AuthenticatedIdentity,
        intent: &DeploymentIntent,
        current_command: CommandIdentity,
        resource: &CapabilitySlotKey,
    ) -> Result<PreparedInstanceDeleteHistory, DeploymentError> {
        original::load(self, identity, intent, current_command, resource).await
    }
    async fn observe_completed_delete(
        &self,
        request: &PreparedInstanceDeleteRequest,
    ) -> Result<PreparedInstanceDeleteCompletedHistory, DeploymentError> {
        worker::observe(self, request).await
    }
}
