use super::super::query;
use recipe_application::{DeploymentError, DeploymentExecutionPreparation};
use release_domain::ReleaseCommandKey;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use std::collections::BTreeSet;

pub async fn reserve(
    tx: &mut Transaction<'_, Postgres>,
    whole: &DeploymentExecutionPreparation,
    extra: ReleaseCommandKey,
) -> Result<(), DeploymentError> {
    let mut commands = BTreeSet::new();
    let mut instances = BTreeSet::new();
    commands.insert(command(extra)?);
    for child in whole.instances() {
        commands.insert(command(child.command_key())?);
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
    // Only a freshly owned transaction may wait here; no row lock exists yet.
    // SQL raw callers use TRY and late NOWAIT, including rechecks after this wait.
    for key in commands {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(&mut **tx)
            .await
            .map_err(|error| query::storage(&error))?;
    }
    for (first, second) in instances {
        sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
            .bind(first)
            .bind(second)
            .execute(&mut **tx)
            .await
            .map_err(|error| query::storage(&error))?;
    }
    Ok(())
}
fn command(key: ReleaseCommandKey) -> Result<i64, DeploymentError> {
    Ok(i64::from_be_bytes(
        *key.as_bytes()
            .first_chunk::<8>()
            .ok_or(DeploymentError::Serialization)?,
    ))
}
