//! Exact fresh scalar mutations through worker-only profile-qualified SQL.
//!
//! Trusted caller detach retains baseline semantics; these methods do not mint
//! physical absence or owned-recipe cleanup evidence. No canonical snapshot is
//! written and the same resource table remains authoritative.

use runtime_types::{RunId, VolumeId};
use time::OffsetDateTime;
use volume_trait::{VolumeError, VolumeLease};

use crate::{PostgresVolumeMetadataRepository, errors::metadata, models::LeaseRow};

pub async fn acquire(
    repository: &PostgresVolumeMetadataRepository,
    volume: VolumeId,
    run: RunId,
    host: &str,
    now: OffsetDateTime,
    expires: OffsetDateTime,
) -> Result<VolumeLease, VolumeError> {
    sqlx::query_as::<_, LeaseRow>("SELECT * FROM legacy_scalar_acquire($1,$2,$3,$4,$5)")
        .bind(volume.as_uuid())
        .bind(run.as_uuid())
        .bind(host)
        .bind(now)
        .bind(expires)
        .fetch_one(&repository.pool)
        .await
        .map_err(metadata)?
        .try_into()
}

pub async fn refresh(
    repository: &PostgresVolumeMetadataRepository,
    lease: &VolumeLease,
    now: OffsetDateTime,
    expires: OffsetDateTime,
    attached: bool,
) -> Result<VolumeLease, VolumeError> {
    sqlx::query_as::<_, LeaseRow>("SELECT * FROM legacy_scalar_refresh($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(lease.id.as_uuid())
        .bind(lease.volume_id.as_uuid())
        .bind(lease.run_id.as_uuid())
        .bind(&lease.host_id)
        .bind(lease.fencing_token)
        .bind(now)
        .bind(expires)
        .bind(attached)
        .fetch_one(&repository.pool)
        .await
        .map_err(metadata)?
        .try_into()
}

pub async fn active(
    repository: &PostgresVolumeMetadataRepository,
    run: RunId,
    host: &str,
) -> Result<Option<VolumeLease>, VolumeError> {
    sqlx::query_as::<_, LeaseRow>("SELECT * FROM legacy_scalar_active($1,$2)")
        .bind(run.as_uuid())
        .bind(host)
        .fetch_optional(&repository.pool)
        .await
        .map_err(metadata)?
        .map(TryInto::try_into)
        .transpose()
}

pub async fn stale(
    repository: &PostgresVolumeMetadataRepository,
    now: OffsetDateTime,
) -> Result<Vec<VolumeLease>, VolumeError> {
    // Action inventory only; omitted historical/canonical rows remain globally
    // held and must never be interpreted as resource absence.
    sqlx::query_as::<_, LeaseRow>("SELECT * FROM legacy_scalar_stale($1)")
        .bind(now)
        .fetch_all(&repository.pool)
        .await
        .map_err(metadata)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
}

pub async fn recover(
    repository: &PostgresVolumeMetadataRepository,
    lease: &VolumeLease,
) -> Result<(), VolumeError> {
    sqlx::query("SELECT legacy_scalar_begin_recovery($1,$2,$3,$4,$5)")
        .bind(lease.id.as_uuid())
        .bind(lease.volume_id.as_uuid())
        .bind(lease.run_id.as_uuid())
        .bind(&lease.host_id)
        .bind(lease.fencing_token)
        .execute(&repository.pool)
        .await
        .map_err(metadata)?;
    Ok(())
}

pub async fn release(
    repository: &PostgresVolumeMetadataRepository,
    lease: &VolumeLease,
    recovering: bool,
) -> Result<(), VolumeError> {
    sqlx::query("SELECT release_legacy_scalar_lease_after_detach($1,$2,$3,$4,$5,$6)")
        .bind(lease.id.as_uuid())
        .bind(lease.volume_id.as_uuid())
        .bind(lease.run_id.as_uuid())
        .bind(&lease.host_id)
        .bind(lease.fencing_token)
        .bind(recovering)
        .execute(&repository.pool)
        .await
        .map_err(metadata)?;
    Ok(())
}
