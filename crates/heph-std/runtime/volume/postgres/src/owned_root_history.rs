use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use uuid::Uuid;
use volume_trait::{
    OwnedVolumeRootHistory, OwnedVolumeRootHistoryConflict, VolumeError,
    VolumeRootHistoryRepository, VolumeRootNamespaceId,
};

use crate::{PostgresVolumeMetadataRepository, errors::metadata, owned_provisioning::storage};

#[derive(sqlx::FromRow)]
struct RootIdentityRow {
    root_path: String,
    owner_namespace: Uuid,
}

#[async_trait]
impl VolumeRootHistoryRepository for PostgresVolumeMetadataRepository {
    async fn owned_root_history(
        &self,
        configured_host: &str,
        configured_root: &Path,
    ) -> Result<OwnedVolumeRootHistory, VolumeError> {
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        // An actor's project-filtered empty result can never prove no history.
        storage::role(&mut tx, "hephaestus_worker").await?;
        let root = checked_root(configured_host, configured_root)?;
        let rows = sqlx::query_as::<_, RootIdentityRow>(
            "SELECT DISTINCT root_path,owner_namespace FROM volume_owned_backing_purposes WHERE host_id=$1 ORDER BY root_path,owner_namespace LIMIT 2",
        )
        .bind(configured_host)
        .fetch_all(&mut *tx)
        .await
        .map_err(metadata)?;
        let result = match rows.as_slice() {
            [] => OwnedVolumeRootHistory::NoOwnedHistory,
            [row] if row.root_path == root => OwnedVolumeRootHistory::Existing {
                namespace: VolumeRootNamespaceId::from_uuid(row.owner_namespace)
                    .map_err(metadata)?,
            },
            [_] => OwnedVolumeRootHistory::Conflict {
                code: OwnedVolumeRootHistoryConflict::RootRedirect,
            },
            _ => OwnedVolumeRootHistory::Conflict {
                code: OwnedVolumeRootHistoryConflict::MultipleOwnerIdentities,
            },
        };
        tx.commit().await.map_err(metadata)?;
        Ok(result)
    }
}

fn checked_root<'a>(host: &str, root: &'a Path) -> Result<&'a str, VolumeError> {
    let text = root.to_str().ok_or(VolumeError::IntentConflict)?;
    if host.is_empty()
        || host.len() > 128
        || host.chars().any(char::is_control)
        || text.is_empty()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || root.components().collect::<PathBuf>().to_str() != Some(text)
    {
        Err(VolumeError::IntentConflict)
    } else {
        Ok(text)
    }
}
