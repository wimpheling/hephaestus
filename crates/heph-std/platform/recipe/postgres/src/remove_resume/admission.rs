#[path = "admission/work.rs"]
mod work;

use super::{authority, context, decoding};
use crate::{PostgresDeploymentRepository, hydration, receipts, repository_error};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AdmissionDisposition, DeploymentAdmission, DeploymentError, DeploymentOperation,
    RemoveDeployment, RemoveResumeAdmission,
};
use serde_json::Value;

impl PostgresDeploymentRepository {
    /// Admits a genuinely new manager while preserving original removal claims.
    ///
    /// Explicit configured support is required; terminal inspection is separate.
    /// No old claim, request, gate, deployment version or provider is mutated.
    ///
    /// # Errors
    /// Denies unsupported configuration, permission loss, stale CAS or unproven history.
    pub async fn admit_remove_resume(
        &self,
        identity: &AuthenticatedIdentity,
        remove: RemoveDeployment,
    ) -> Result<RemoveResumeAdmission, DeploymentError> {
        let config = self
            .remove_resume
            .as_ref()
            .ok_or(DeploymentError::InvalidAction)?;
        remove
            .command
            .validate(identity, DeploymentOperation::Remove)?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(repository_error)?;
        context::reserve(&mut tx, remove.deployment_id).await?;
        sqlx::query("SELECT lock_remove_resume118($1)")
            .bind(remove.deployment_id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(repository_error)?;
        let snapshot = hydration::load(&mut tx, remove.deployment_id).await?;
        context::configured(&mut tx, &snapshot.intent, config).await?;
        authority::all(&mut tx, identity, &snapshot.intent).await?;
        let hash = remove.input_hash()?;
        let prior = receipts::find(&mut tx, remove.command).await?;
        let preparation = crate::execution_preparation::hydration::load(&mut tx, &snapshot.intent)
            .await?
            .ok_or(DeploymentError::ReconciliationRequired)?
            .preparation;
        let roots = decoding::Roots {
            intent: &snapshot.intent,
            preparation: &preparation,
            config,
        };
        let current_data: Value = sqlx::query_scalar("SELECT remove_resume_context118($1)")
            .bind(remove.deployment_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(decoding::storage)?;
        let (receipt, originals, disposition) = if let Some(prior) = prior {
            let (receipt, originals) = work::restore_receipt(
                &mut tx,
                remove,
                &snapshot,
                &roots,
                &prior,
                hash,
                &current_data,
            )
            .await?;
            (receipt, originals, AdmissionDisposition::Resume)
        } else {
            let (receipt, originals) = work::new_receipt(
                &mut tx,
                identity,
                remove,
                &snapshot,
                &roots,
                hash,
                &current_data,
            )
            .await?;
            (receipt, originals, AdmissionDisposition::Created)
        };
        receipts::attempt(
            &mut tx,
            identity,
            &receipt,
            snapshot.intent.project_id(),
            disposition,
        )
        .await?;
        let admitted = DeploymentAdmission {
            disposition,
            snapshot,
            receipt,
        };
        let result = if originals.detach.is_empty() && originals.terminal.is_empty() {
            RemoveResumeAdmission::with_pending_instance_drains(
                admitted,
                originals.continuations,
                originals.closures,
                originals.pending,
            )?
        } else {
            RemoveResumeAdmission::with_instance_terminal_history(
                admitted,
                originals.continuations,
                originals.closures,
                originals.pending,
                originals.detach,
                originals.terminal,
                config,
            )?
        };
        tx.commit().await.map_err(repository_error)?;
        Ok(result)
    }
}
