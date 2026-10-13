//! Pre-project reservation for the exact checked original whole preparation.
//! Mutex labels serialize births; they never establish ownership or authority.

use std::collections::BTreeSet;

use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentExecutionPreparation, DeploymentId,
};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

use super::{PostgresExecutionPreparationRepository, authority, hydration};
use crate::repository_error;

pub async fn prospective(
    adapter: &PostgresExecutionPreparationRepository,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    deployment: DeploymentId,
) -> Result<DeploymentExecutionPreparation, DeploymentError> {
    authority::install_open(tx, deployment).await?;
    // This loader performs no row locking. The caller has already checked the
    // current actor and target ProjectManage. All checks repeat under row locks.
    let snapshot = crate::hydration::load(tx, deployment).await?;
    let original = authority::original(tx, &snapshot.intent).await?;
    if command != original {
        return Err(DeploymentError::InputConflict);
    }
    let declaration = recipe_domain::parse_recipe(snapshot.intent.declaration_toml().as_bytes())?;
    adapter
        .repository
        .install_authority(tx, identity, &declaration, &snapshot.intent)
        .await?;
    let existing = hydration::load(tx, &snapshot.intent).await?;
    if let Some(stored) = &existing {
        stored
            .preparation
            .validate_configuration(adapter.profile, &adapter.platform)?;
    }
    let sources = authority::sources(tx, &declaration, &snapshot.intent, &adapter.platform).await?;
    let desired = DeploymentExecutionPreparation::new(
        &snapshot.intent,
        original,
        adapter.profile,
        adapter.platform.clone(),
        &sources,
    )?;
    if existing
        .as_ref()
        .is_some_and(|stored| stored.preparation != desired)
    {
        return Err(DeploymentError::InputConflict);
    }
    reserve(tx, &desired).await?;
    Ok(desired)
}

async fn reserve(
    tx: &mut Transaction<'_, Postgres>,
    preparation: &DeploymentExecutionPreparation,
) -> Result<(), DeploymentError> {
    // This is the adapter's freshly owned transaction, before ANY row lock.
    // Raw protected SQL callers use try-locks instead; borrowed transactions
    // must never use this blocking protocol. All authority repeats afterwards.
    block_before_project(tx, preparation).await?;
    let children: Vec<Vec<u8>> = preparation
        .instances()
        .iter()
        .map(serde_json::to_vec)
        .collect::<Result<_, _>>()
        .map_err(|_| DeploymentError::Serialization)?;
    sqlx::query("SELECT reserve_recipe_execution_preparation($1,$2,$3,$4)")
        .bind(preparation.deployment_id().as_uuid())
        .bind(preparation.command().id().as_uuid())
        .bind(preparation.canonical_bytes()?)
        .bind(children)
        .execute(&mut **tx)
        .await
        .map_err(repository_error)?;
    Ok(())
}

async fn block_before_project(
    tx: &mut Transaction<'_, Postgres>,
    preparation: &DeploymentExecutionPreparation,
) -> Result<(), DeploymentError> {
    let mut commands = BTreeSet::new();
    let mut instances = BTreeSet::new();
    for child in preparation.instances() {
        commands.insert(i64::from_be_bytes(
            *child
                .command_key()
                .as_bytes()
                .first_chunk::<8>()
                .ok_or(DeploymentError::Serialization)?,
        ));
        let mut digest = Sha256::new();
        digest.update(b"hephaestus-recipe113-instance-birth-v1\0");
        digest.update(child.instance_id().as_uuid().as_bytes());
        let bytes: [u8; 32] = digest.finalize().into();
        let first = *bytes
            .first_chunk::<4>()
            .ok_or(DeploymentError::Serialization)?;
        let second = *bytes
            .get(4..)
            .and_then(|part| part.first_chunk::<4>())
            .ok_or(DeploymentError::Serialization)?;
        instances.insert((i32::from_be_bytes(first), i32::from_be_bytes(second)));
    }
    for key in commands {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(&mut **tx)
            .await
            .map_err(repository_error)?;
    }
    for (first, second) in instances {
        sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
            .bind(first)
            .bind(second)
            .execute(&mut **tx)
            .await
            .map_err(repository_error)?;
    }
    Ok(())
}
