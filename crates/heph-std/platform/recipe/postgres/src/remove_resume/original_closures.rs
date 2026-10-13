//! Current-auth comparison of original normal Remove closures, without admission.
use super::{context, decoding};
use crate::{
    PostgresDeploymentRepository, execution_preparation, hydration, receipts, repository_error,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentIntent, DeploymentLifecycle, DeploymentOperation,
    OriginalInstanceClosure, RemoveDeployment,
};

impl PostgresDeploymentRepository {
    /// Reads full original closures/CAS under CURRENT configured ALL-owned rights.
    ///
    /// A normal original command stays normal; no resume admission is fabricated.
    /// Positive zero closures requires a complete protected context, never missing
    /// or filtered history. Saved CAS values are comparison data, not permission.
    ///
    /// # Errors
    /// Denies wrong role/config/command, foreign or incomplete whole intent/history,
    /// current permission loss, contradictory published closures or rollback failure.
    pub async fn load_remove_original_closures(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        intent: &DeploymentIntent,
    ) -> Result<Vec<OriginalInstanceClosure>, DeploymentError> {
        command.validate(identity, DeploymentOperation::Remove)?;
        let config = self
            .remove_resume
            .as_ref()
            .ok_or(DeploymentError::InvalidAction)?;
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await
            .map_err(repository_error)?;
        let result = async {
            let role: String = sqlx::query_scalar("SELECT current_user::text").fetch_one(&mut *tx).await.map_err(repository_error)?;
            if role != "hephaestus_app" { return Err(DeploymentError::AuthorizationDenied); }
            sqlx::query("SELECT set_config('search_path','pg_catalog,public,pg_temp',true), set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)")
                .bind(identity.user_id.to_string()).bind(identity.request_id.to_string()).bind(identity.idempotency_id.to_string()).execute(&mut *tx).await.map_err(repository_error)?;
            // Existing protected front repeats actual active caller + ALL-owned
            // rights before returning complete original/Pending/terminal history.
            let data: serde_json::Value = sqlx::query_scalar("SELECT remove_resume_context118($1)")
                .bind(intent.id().as_uuid()).fetch_one(&mut *tx).await.map_err(decoding::storage)?;
            let snapshot = hydration::load(&mut tx, intent.id()).await?;
            if snapshot.intent != *intent || !matches!(snapshot.lifecycle, DeploymentLifecycle::Removing | DeploymentLifecycle::RecoveryRequired) { return Err(DeploymentError::IntentMismatch); }
            context::configured(&mut tx, intent, config).await?;
            context::command_configuration(&mut tx, command, config).await?;
            let receipt = receipts::find(&mut tx, command).await?.ok_or(DeploymentError::Unavailable)?;
            if !matches!(receipt.receipt_lifecycle.as_str(), "removing" | "recovery_required") { return Err(DeploymentError::ReconciliationRequired); }
            let input = match context::current_input(&mut tx, command, &snapshot, Some(config)).await? {
                Some(input) => input,
                None => RemoveDeployment { command, deployment_id: intent.id(), expected_version: hydration::unsigned(receipt.receipt_version)?.checked_sub(1).ok_or(DeploymentError::ReconciliationRequired)? }.input_hash()?,
            };
            receipts::restore(&receipt, command, &snapshot, input)?;
            let event_valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_deployment_commands command JOIN application_events event ON event.id=command.event_id JOIN product_event_outbox outbox ON outbox.event_id=event.id WHERE command.id=$1 AND command.actor_id=$2 AND command.operation='remove' AND event.actor_id=command.actor_id AND event.request_id=command.first_request_id AND event.occurrence_id=command.idempotency_id AND event.scope_kind='project' AND event.scope_id=command.project_id AND event.aggregate_type='project' AND event.aggregate_id=command.project_id AND event.event_type='project.changed' AND event.change_kind='updated' AND event.schema_version=1 AND event.actor_type='user' AND event.cursor=command.event_cursor AND event.aggregate_version=command.event_aggregate_version)")
                .bind(command.id().as_uuid()).bind(identity.user_id.as_uuid()).fetch_one(&mut *tx).await.map_err(repository_error)?;
            if !event_valid { return Err(DeploymentError::ReconciliationRequired); }
            let prepared = execution_preparation::hydration::load(&mut tx, intent).await?.ok_or(DeploymentError::ReconciliationRequired)?.preparation;
            let roots = decoding::Roots { intent, preparation: &prepared, config };
            let decoded = decoding::load(&mut tx, &data, &roots).await?;
            // The existing checked Current constructor accounts for EVERY active
            // and terminal/detach original, including positive inactive Applied.
            let current = decoding::current(decoded, &snapshot, command)?;
            Ok(current.closures)
        }.await;
        let rollback = tx.rollback().await.map_err(repository_error);
        match result {
            Err(primary) => Err(primary),
            Ok(value) => rollback.map(|()| value),
        }
    }
}
