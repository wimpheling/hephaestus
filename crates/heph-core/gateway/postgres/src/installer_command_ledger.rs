//! Idempotent command ledger operations for gateway installation.

use super::{GatewayInstallError, InstallGatewayManifest, InstalledGateway};
use gateway_domain::{GatewayId, GatewayRevisionId};
use identity_domain::AuthenticatedIdentity;
use release_domain::ReleaseCommandKey;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct InstallationCommandRow {
    operation: String,
    project_id: Uuid,
    repository_id: Uuid,
    release_id: Uuid,
    actor_id: Uuid,
}

#[derive(sqlx::FromRow)]
pub struct InstallationCommandResultRow {
    pub gateway_id: Uuid,
    pub revision_id: Uuid,
}

pub fn installation_command_key(identity: &AuthenticatedIdentity) -> ReleaseCommandKey {
    ReleaseCommandKey::derive(
        "install_release_gateways",
        &[identity.idempotency_id.as_uuid().as_bytes()],
    )
}

pub async fn claim_installation_command(
    tx: &mut Transaction<'_, Postgres>,
    key: ReleaseCommandKey,
    identity: &AuthenticatedIdentity,
    command: &InstallGatewayManifest,
) -> Result<Option<Vec<InstalledGateway>>, GatewayInstallError> {
    let inserted = sqlx::query(
        "INSERT INTO gateway_install_commands
             (command_key, operation, project_id, repository_id, release_id,
              actor_id, request_id)
         VALUES ($1, 'install_release_gateways', $2, $3, $4, $5, $6)
         ON CONFLICT (command_key) DO NOTHING",
    )
    .bind(key.as_bytes().as_slice())
    .bind(command.project_id.as_uuid())
    .bind(command.repository_id.as_uuid())
    .bind(
        command
            .release_id
            .ok_or(GatewayInstallError::Unavailable)?
            .as_uuid(),
    )
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    if inserted.rows_affected() == 1 {
        return Ok(None);
    }
    let stored = sqlx::query_as::<_, InstallationCommandRow>(
        "SELECT operation, project_id, repository_id, release_id, actor_id
         FROM gateway_install_commands WHERE command_key = $1",
    )
    .bind(key.as_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(GatewayInstallError::Unavailable)?;
    let release_id = command.release_id.ok_or(GatewayInstallError::Unavailable)?;
    if stored.operation != "install_release_gateways"
        || stored.project_id != command.project_id.as_uuid()
        || stored.repository_id != command.repository_id.as_uuid()
        || stored.release_id != release_id.as_uuid()
        || stored.actor_id != identity.user_id.as_uuid()
    {
        return Err(GatewayInstallError::Conflict);
    }
    let results = sqlx::query_as::<_, InstallationCommandResultRow>(
        "SELECT gateway_id, revision_id
         FROM gateway_install_command_results
         WHERE command_key = $1 ORDER BY ordinal",
    )
    .bind(key.as_bytes().as_slice())
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|row| InstalledGateway {
        gateway_id: GatewayId::from_uuid(row.gateway_id),
        revision_id: GatewayRevisionId::from_uuid(row.revision_id),
    })
    .collect();
    Ok(Some(results))
}
