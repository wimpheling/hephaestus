use super::types::ReleaseError;
use uuid::Uuid;

pub fn map_build_error(error: crate::build::BuildError) -> ReleaseError {
    match error {
        crate::build::BuildError::NotFound => ReleaseError::NotFound,
        crate::build::BuildError::InvalidStoredData => ReleaseError::InvalidStoredData,
        crate::build::BuildError::Serialization(error) => ReleaseError::Serialization(error),
        crate::build::BuildError::Persistence(error) => ReleaseError::Persistence(error),
        crate::build::BuildError::FailedPrecondition => ReleaseError::FailedPrecondition,
    }
}

pub async fn ensure_draft(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<(), ReleaseError> {
    let state = sqlx::query_scalar::<_, String>("SELECT state FROM releases WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(ReleaseError::Persistence)?
        .ok_or(ReleaseError::NotFound)?;
    if state == "draft" {
        Ok(())
    } else {
        Err(ReleaseError::FailedPrecondition)
    }
}
