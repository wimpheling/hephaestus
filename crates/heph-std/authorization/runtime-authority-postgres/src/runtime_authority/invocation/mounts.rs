//! Typed Invocation mounts use current full fences and delegation, not kind alone.

use super::super::{agent_repository::PgRuntimeSessionRepository, common::storage};
use runtime_authority::RuntimeAuthorityError;
use runtime_types::RunId;
use volume_domain::VolumeMountScope;

impl PgRuntimeSessionRepository {
    pub(in crate::runtime_authority) async fn invocation_mount_authorized(
        &self,
        run: RunId,
        scope: &VolumeMountScope,
    ) -> Result<bool, RuntimeAuthorityError> {
        sqlx::query_scalar(
            "SELECT invocation119_typed_mount_authorized($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(run.as_uuid())
        .bind(self.invocation_config_bytes()?)
        .bind(scope.instance_id().as_uuid())
        .bind(scope.revision_id().as_uuid())
        .bind(scope.release_agent_id().as_uuid())
        .bind(scope.slot().as_str())
        .bind(scope.volume_id().as_uuid())
        .bind(scope.access_mode().as_str())
        .bind(scope.release_contract_hash().as_bytes().as_slice())
        .fetch_one(&self.pool)
        .await
        .map_err(storage)
    }
}
