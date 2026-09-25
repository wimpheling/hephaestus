//! Release artifact metadata loading for gateway launch adapters.

use super::{GatewayEdgeError, GatewayReleaseArtifact, GatewayReleaseArtifactKind};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct GatewayReleaseArtifactRow {
    path: String,
    kind: String,
    mode: i32,
    content_hash: Vec<u8>,
    size_bytes: i64,
    storage_key: Uuid,
}

pub async fn gateway_release_artifacts(
    pool: &PgPool,
    release_id: Uuid,
) -> Result<Vec<GatewayReleaseArtifact>, GatewayEdgeError> {
    let rows = sqlx::query_as::<_, GatewayReleaseArtifactRow>(
        "SELECT path, kind, mode, content_hash, size_bytes, storage_key
           FROM release_artifacts
          WHERE release_id = $1
          ORDER BY path",
    )
    .bind(release_id)
    .fetch_all(pool)
    .await
    .map_err(|_| GatewayEdgeError::Unavailable)?;
    rows.into_iter().map(gateway_release_artifact).collect()
}

fn gateway_release_artifact(
    row: GatewayReleaseArtifactRow,
) -> Result<GatewayReleaseArtifact, GatewayEdgeError> {
    let content_hash: [u8; 32] = row
        .content_hash
        .try_into()
        .map_err(|_| GatewayEdgeError::Unavailable)?;
    let mode = u32::try_from(row.mode).map_err(|_| GatewayEdgeError::Unavailable)?;
    let size_bytes = u64::try_from(row.size_bytes).map_err(|_| GatewayEdgeError::Unavailable)?;
    let kind = match row.kind.as_str() {
        "executable" => GatewayReleaseArtifactKind::Executable,
        "file" => GatewayReleaseArtifactKind::File,
        "manifest" => GatewayReleaseArtifactKind::Manifest,
        _ => return Err(GatewayEdgeError::Unavailable),
    };
    Ok(GatewayReleaseArtifact {
        path: row.path,
        kind,
        mode,
        content_hash,
        size_bytes,
        storage_key: row.storage_key,
    })
}
