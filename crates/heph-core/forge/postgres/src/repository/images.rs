use super::helpers::storage;
use forge_domain::RepositoryId;
use forge_service::ForgeRepositoryError;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct ResolvedImageRow {
    pub catalog_image_id: Option<Uuid>,
    pub repository_image_id: Option<Uuid>,
    pub key: String,
    pub image_reference: String,
}

pub async fn resolve_image(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    selection: &agent_config::ImageSelection,
) -> Result<ResolvedImageRow, ForgeRepositoryError> {
    let image = match (&selection.key, &selection.project_image) {
        (Some(key), None) => sqlx::query_as::<_, ResolvedImageRow>(
            "SELECT id AS catalog_image_id, NULL::uuid AS repository_image_id,
                    key, image_reference
               FROM oci_images
              WHERE key = $1 AND availability_state = 'available' AND role = 'execution'",
        )
        .bind(key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?,
        (None, Some(key)) => sqlx::query_as::<_, ResolvedImageRow>(
            "SELECT NULL::uuid AS catalog_image_id, image.id AS repository_image_id,
                    image.key, image.image_reference
               FROM repository_oci_image_definitions AS image
               JOIN repositories AS repository ON repository.id = $1
              WHERE image.project_id = repository.project_id
                AND image.key = $2
                AND image.status = 'ready'
              ORDER BY image.updated_at DESC, image.id DESC
              LIMIT 1
              FOR SHARE OF image",
        )
        .bind(repository_id.as_uuid())
        .bind(key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?,
        _ => None,
    };
    image.ok_or(ForgeRepositoryError::InvalidMetadata(
        "selected OCI image is unavailable",
    ))
}

pub fn hex_digest(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}
