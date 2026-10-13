//! Explicit owned-profile live checks. Ordinary issuance remains Provisioning-only.

pub(super) mod ceiling;
mod context;
mod lookup;
pub(super) mod session;
mod source;

pub use source::LiveRunSourceContext;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use capability_domain::AuthorizationSnapshot;
use runtime_authority::{LiveRunSnapshotRequest, LiveRunSnapshotResolver, RuntimeAuthorityError};
use time::OffsetDateTime;

use super::{agent_repository::PgRuntimeSessionRepository, common::storage};

#[async_trait]
impl LiveRunSnapshotResolver for PgRuntimeSessionRepository {
    async fn resolve_live_snapshot(
        &self,
        request: &LiveRunSnapshotRequest,
        authorization_model_version: &str,
        now: OffsetDateTime,
    ) -> Result<AuthorizationSnapshot, RuntimeAuthorityError> {
        if !self.canonical_live_snapshots && self.invocation_configuration.is_none() {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        if self.qualified_invocation_run(request.pins().run()).await? {
            return self
                .invocation_live_snapshot(request, authorization_model_version, now)
                .await;
        }
        if !self.canonical_live_snapshots {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        context::ensure_worker(&mut tx).await?;
        let source = context::source(&mut tx, request).await?;
        if source.actor_id.is_nil() || source.request_id.is_nil() {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),set_config('hephaestus.request_id',$2,true)")
            .bind(source.actor_id.to_string()).bind(source.request_id.to_string())
            .execute(&mut *tx).await.map_err(storage)?;
        let source_allowed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=$1 AND status='active')
                AND check_permission('user',$1::text,'can_use','release_agent',$2::text)=1",
        )
        .bind(source.actor_id)
        .bind(request.pins().release_agent())
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
        if !source_allowed {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        let snapshot =
            ceiling::resolve(&mut tx, request.pins(), authorization_model_version).await?;
        if let Some(expected) = request.ceiling() {
            session::validate(
                &mut tx,
                request.pins(),
                *expected,
                snapshot.normalized_hash(),
                now,
            )
            .await?;
        }
        if !ceiling::operations_authorized(&mut tx, &snapshot).await? {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        tx.commit().await.map_err(storage)?;
        Ok(snapshot)
    }
}
