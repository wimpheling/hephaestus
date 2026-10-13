use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentExecutionPreparation, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, RemoveDeployment,
};
use sqlx::{Postgres, Transaction};

use super::super::{authority as preparation_authority, hydration};
use crate::{
    execution_preparation::PostgresExecutionPreparationRepository, receipts, repository_error,
};

pub async fn current(
    adapter: &PostgresExecutionPreparationRepository,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    snapshot: &DeploymentSnapshot,
) -> Result<(), DeploymentError> {
    let row = receipts::find(tx, command)
        .await?
        .ok_or(DeploymentError::Unavailable)?;
    let resumed = if command.operation() == DeploymentOperation::Remove {
        crate::remove_resume::context::current_input(
            tx,
            command,
            snapshot,
            adapter.repository.remove_resume.as_ref(),
        )
        .await?
    } else {
        None
    };
    let resumed_receipt = resumed.is_some();
    let input = if let Some(input) = resumed {
        input
    } else {
        match command.operation() {
            DeploymentOperation::Install => snapshot.intent.input_hash(),
            DeploymentOperation::Remove => RemoveDeployment {
                command,
                deployment_id: snapshot.intent.id(),
                expected_version: crate::hydration::unsigned(row.receipt_version)?
                    .checked_sub(1)
                    .ok_or(DeploymentError::ReconciliationRequired)?,
            }
            .input_hash()?,
        }
    };
    receipts::restore(&row, command, snapshot, input)?;
    admission(tx, command).await?;
    adapter
        .repository
        .cleanup_authority(tx, identity, &snapshot.intent)
        .await?;
    if command.operation() == DeploymentOperation::Remove {
        if !matches!(
            snapshot.lifecycle,
            DeploymentLifecycle::Removing
                | DeploymentLifecycle::RecoveryRequired
                | DeploymentLifecycle::Removed
        ) || !(row.receipt_lifecycle == "removing"
            || (resumed_receipt && row.receipt_lifecycle == "recovery_required"))
        {
            return Err(DeploymentError::ReconciliationRequired);
        }
        return Ok(());
    }
    install(adapter, tx, identity, command, snapshot).await
}

async fn install(
    adapter: &PostgresExecutionPreparationRepository,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    snapshot: &DeploymentSnapshot,
) -> Result<(), DeploymentError> {
    preparation_authority::install_open(tx, snapshot.intent.id()).await?;
    let original = preparation_authority::original(tx, &snapshot.intent).await?;
    if original != command {
        return Err(DeploymentError::InputConflict);
    }
    let declaration = recipe_domain::parse_recipe(snapshot.intent.declaration_toml().as_bytes())?;
    adapter
        .repository
        .install_authority(tx, identity, &declaration, &snapshot.intent)
        .await?;
    let stored = hydration::load(tx, &snapshot.intent)
        .await?
        .ok_or(DeploymentError::ReconciliationRequired)?;
    stored
        .preparation
        .validate_configuration(adapter.profile, &adapter.platform)?;
    let sources =
        preparation_authority::sources(tx, &declaration, &snapshot.intent, &adapter.platform)
            .await?;
    let desired = DeploymentExecutionPreparation::new(
        &snapshot.intent,
        original,
        adapter.profile,
        adapter.platform.clone(),
        &sources,
    )?;
    if stored.preparation != desired {
        return Err(DeploymentError::InputConflict);
    }
    Ok(())
}

async fn admission(
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
) -> Result<(), DeploymentError> {
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM recipe_deployment_commands command
          JOIN application_events event ON event.id=command.event_id
          JOIN product_event_outbox outbox ON outbox.event_id=event.id
         WHERE command.id=$1 AND event.scope_kind='project' AND event.scope_id=command.project_id
           AND event.aggregate_type='project' AND event.aggregate_id=command.project_id
           AND event.event_type='project.changed' AND event.change_kind='updated'
           AND event.occurrence_id=command.idempotency_id AND event.actor_type='user'
           AND event.actor_id=command.actor_id AND event.request_id=command.first_request_id
           AND event.cursor=command.event_cursor AND event.aggregate_version=command.event_aggregate_version)",
    ).bind(command.id().as_uuid()).fetch_one(&mut **tx).await.map_err(repository_error)?;
    if !valid {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
