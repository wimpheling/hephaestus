use crate::{
    PostgresDeploymentRepository, execution_transitions, receipts, repository_error,
    rows::ReceiptRow,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, CommandReceipt, DeploymentError, DeploymentId, DeploymentLifecycle,
    DeploymentOperation, InstallProgress, RemovalProgress,
};
use release_domain::ContentHash;

pub async fn exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    command: CommandIdentity,
) -> Result<bool, DeploymentError> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_command_results WHERE command_id=$1)")
        .bind(command.id().as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(repository_error)
}

impl PostgresDeploymentRepository {
    /// Commits a distinct terminal receipt after every resource satisfies the policy.
    ///
    /// # Errors
    /// Rejects denied access, changed completion input, stale CAS, or incomplete resources.
    pub async fn finish_command(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        deployment_id: DeploymentId,
        expected_version: u64,
    ) -> Result<CommandReceipt, DeploymentError> {
        let mut context = self
            .execution_context(identity, command, deployment_id, false)
            .await?;
        let fingerprint = serde_json::to_vec(&(command, deployment_id, expected_version))
            .map(|bytes| ContentHash::digest(&bytes))
            .map_err(|_| DeploymentError::Serialization)?;
        let saved: Option<(Vec<u8>,)> = sqlx::query_as(
            "SELECT finish_fingerprint FROM recipe_command_results WHERE command_id=$1",
        )
        .bind(command.id().as_uuid())
        .fetch_optional(&mut *context.tx)
        .await
        .map_err(repository_error)?;
        let admission = receipts::find(&mut context.tx, command)
            .await?
            .ok_or(DeploymentError::Unavailable)?;
        let input_hash = crate::hydration::digest(&admission.input_hash)?;
        if let Some((stored,)) = saved {
            if stored.as_slice() != fingerprint.as_bytes() {
                return Err(DeploymentError::InputConflict);
            }
            let receipt = restore_terminal(&mut context, command, input_hash).await?;
            context.tx.commit().await.map_err(repository_error)?;
            return Ok(receipt);
        }
        if context.snapshot.version != expected_version {
            return Err(DeploymentError::StaleClaim);
        }
        let ready = context.snapshot.resources.values().all(|state| {
            state.active_attempt.is_none()
                && match command.operation() {
                    DeploymentOperation::Install => state.install == InstallProgress::Ready,
                    DeploymentOperation::Remove => matches!(
                        state.removal,
                        RemovalProgress::Retained | RemovalProgress::Deleted
                    ),
                }
        });
        if !ready
            || (command.operation() == DeploymentOperation::Install && context.cleanup)
            || (command.operation() == DeploymentOperation::Remove && !context.cleanup)
        {
            return Err(DeploymentError::InvalidAction);
        }
        let lifecycle = match command.operation() {
            DeploymentOperation::Install => DeploymentLifecycle::Installed,
            DeploymentOperation::Remove => DeploymentLifecycle::Removed,
        };
        let version = expected_version
            .checked_add(1)
            .ok_or(DeploymentError::StaleClaim)?;
        let transition = execution_transitions::transition_id(command.id().as_uuid(), "finished");
        let (event_id, cursor, aggregate_version) = execution_transitions::event(
            &mut context.tx,
            identity,
            transition,
            context.organization,
            context.snapshot.intent.project_id(),
        )
        .await?;
        sqlx::query("INSERT INTO recipe_execution_transitions (id, command_id, operation_id, deployment_id, project_id, kind,
            fingerprint, actor_id, request_id, before_deployment_version, after_deployment_version, before_lifecycle,
            after_lifecycle, event_id, event_cursor, event_aggregate_version)
            VALUES ($1,$2,$2,$3,$4,'finished',$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)")
            .bind(transition).bind(command.id().as_uuid()).bind(deployment_id.as_uuid()).bind(context.snapshot.intent.project_id().as_uuid())
            .bind(fingerprint.as_bytes().as_slice()).bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid())
            .bind(i64::try_from(expected_version).map_err(|_| DeploymentError::StaleClaim)?)
            .bind(i64::try_from(version).map_err(|_| DeploymentError::StaleClaim)?)
            .bind(context.snapshot.lifecycle.as_str()).bind(lifecycle.as_str())
            .bind(event_id).bind(cursor).bind(aggregate_version).execute(&mut *context.tx).await.map_err(repository_error)?;
        let updated = sqlx::query("UPDATE recipe_deployments SET lifecycle=$2, version=version+1 WHERE id=$1 AND version=$3")
            .bind(deployment_id.as_uuid()).bind(lifecycle.as_str())
            .bind(i64::try_from(expected_version).map_err(|_| DeploymentError::StaleClaim)?)
            .execute(&mut *context.tx).await.map_err(repository_error)?;
        if updated.rows_affected() != 1 {
            return Err(DeploymentError::StaleClaim);
        }
        sqlx::query("INSERT INTO recipe_command_results (command_id, deployment_id, project_id, input_hash, finish_fingerprint,
            lifecycle, version, event_id, event_cursor, event_aggregate_version, actor_id, request_id, transition_id)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)")
            .bind(command.id().as_uuid()).bind(deployment_id.as_uuid()).bind(context.snapshot.intent.project_id().as_uuid())
            .bind(input_hash.as_bytes().as_slice()).bind(fingerprint.as_bytes().as_slice()).bind(lifecycle.as_str())
            .bind(i64::try_from(version).map_err(|_| DeploymentError::StaleClaim)?)
            .bind(event_id).bind(cursor).bind(aggregate_version).bind(identity.user_id.as_uuid())
            .bind(identity.request_id.as_uuid()).bind(transition).execute(&mut *context.tx).await.map_err(repository_error)?;
        let receipt = restore_terminal(&mut context, command, input_hash).await?;
        context.tx.commit().await.map_err(repository_error)?;
        Ok(receipt)
    }
}

async fn restore_terminal(
    context: &mut crate::effect_context::Context<'_>,
    command: CommandIdentity,
    input_hash: ContentHash,
) -> Result<CommandReceipt, DeploymentError> {
    let row: ReceiptRow = sqlx::query_as(
        "SELECT command_id AS id, deployment_id, project_id, input_hash,
                lifecycle AS receipt_lifecycle, version AS receipt_version, event_id, event_cursor,
                event_aggregate_version FROM recipe_command_results WHERE command_id=$1",
    )
    .bind(command.id().as_uuid())
    .fetch_one(&mut *context.tx)
    .await
    .map_err(repository_error)?;
    receipts::restore(&row, command, &context.snapshot, input_hash)
}
