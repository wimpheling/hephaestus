use builder_catalog_domain::{OciDigest, OciImageReference};
use heph_build::{
    OciImageProductionOutput, OciWorkerStoreError, RepositoryOciImageProvenance,
    RepositoryOciImageSourcePath,
};
use sqlx::{Postgres, Transaction};
use std::time::Duration;
use uuid::Uuid;

pub async fn append_event(
    transaction: &mut Transaction<'_, Postgres>,
    image_id: Uuid,
    change_kind: &str,
    status: &str,
) -> Result<(), OciWorkerStoreError> {
    let event_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT event.event_id
         FROM repository_oci_image_definitions AS definition
         CROSS JOIN LATERAL append_application_event(
            gen_random_uuid(), 'project', definition.project_id, 'project', definition.project_id,
            'project.changed', $2, $3, $1, NULL
         ) AS event
         WHERE definition.id = $1",
    )
    .bind(image_id)
    .bind(change_kind)
    .bind(status)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?;
    event_id.map(|_| ()).ok_or(OciWorkerStoreError::Conflict)
}

pub fn validate_output(
    output: &OciImageProductionOutput,
    provenance: &RepositoryOciImageProvenance,
) -> Result<(), OciWorkerStoreError> {
    if output.image_reference.digest().map_err(invalid)? != output.image_digest
        || output.scan_reference.trim().is_empty()
        || output.scan_reference.len() > 2048
    {
        return Err(OciWorkerStoreError::Conflict);
    }
    provenance.validate().map_err(invalid)
}

pub fn parse_reference(value: String) -> Result<OciImageReference, OciWorkerStoreError> {
    OciImageReference::parse(value).map_err(invalid)
}

pub fn parse_digest(value: String) -> Result<OciDigest, OciWorkerStoreError> {
    OciDigest::parse(value).map_err(invalid)
}

pub fn parse_path(value: String) -> Result<RepositoryOciImageSourcePath, OciWorkerStoreError> {
    RepositoryOciImageSourcePath::parse(value).map_err(invalid)
}

pub fn lease_seconds(lease: Duration) -> Result<i64, OciWorkerStoreError> {
    i64::try_from(lease.as_secs())
        .ok()
        .filter(|seconds| *seconds > 0)
        .ok_or(OciWorkerStoreError::Conflict)
}

pub fn validate_reason(reason: &str) -> Result<(), OciWorkerStoreError> {
    (!reason.trim().is_empty() && reason.len() <= 2048)
        .then_some(())
        .ok_or(OciWorkerStoreError::Conflict)
}

pub fn invalid(error: impl std::error::Error + Send + Sync + 'static) -> OciWorkerStoreError {
    OciWorkerStoreError::Storage(Box::new(error))
}

pub fn storage(error: impl std::error::Error + Send + Sync + 'static) -> OciWorkerStoreError {
    OciWorkerStoreError::Storage(Box::new(error))
}
