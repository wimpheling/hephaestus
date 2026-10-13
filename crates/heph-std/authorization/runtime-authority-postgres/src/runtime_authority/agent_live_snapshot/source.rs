//! Stable original producer identity, not a caller grant or HTTP credential.

use runtime_authority::{LiveRunSnapshotPins, LiveRunSnapshotRequest, RuntimeAuthorityError};
use uuid::Uuid;

use super::context;
use crate::runtime_authority::{agent_repository::PgRuntimeSessionRepository, common::storage};

/// Exact original source identity loaded from immutable owned-run provenance.
///
/// This non-serializable projection grants no Execute, Update, source or mount
/// rights. A trusted app caller must perform fresh live and audited checks.
#[derive(Debug, Clone, Copy)]
pub struct LiveRunSourceContext {
    pins: LiveRunSnapshotPins,
    actor_id: Uuid,
    request_id: Uuid,
}

impl LiveRunSourceContext {
    /// Returns the exact immutable run pins checked with the producer.
    #[must_use]
    pub const fn pins(self) -> LiveRunSnapshotPins {
        self.pins
    }
    /// Returns the recorded original actor identifier, not an authenticated user.
    #[must_use]
    pub const fn actor_id(self) -> Uuid {
        self.actor_id
    }
    /// Returns the recorded producer request identifier.
    #[must_use]
    pub const fn request_id(self) -> Uuid {
        self.request_id
    }
}

impl PgRuntimeSessionRepository {
    /// Loads original actor/request facts through the shared owned phase query.
    ///
    /// Normal requests and provision-consumed update admissions are supported;
    /// mailbox, historical, cleanup-only and unadmitted invocation rows deny.
    /// This read is not a substitute for `resolve_live_snapshot` validation.
    ///
    /// # Errors
    ///
    /// Rejects disabled configuration, non-worker calls, unavailable exact
    /// provenance, malformed principal identifiers, or storage errors.
    pub async fn recorded_live_source_context(
        &self,
        request: &LiveRunSnapshotRequest,
    ) -> Result<LiveRunSourceContext, RuntimeAuthorityError> {
        if !self.canonical_live_snapshots && self.invocation_configuration.is_none() {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        if self.qualified_invocation_run(request.pins().run()).await? {
            let mut tx = self.pool.begin().await.map_err(storage)?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
            let source = self.invocation_source(&mut tx, request).await?;
            tx.commit().await.map_err(storage)?;
            return Ok(LiveRunSourceContext {
                pins: request.pins(),
                actor_id: source.actor_id,
                request_id: source.request_id,
            });
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
        tx.commit().await.map_err(storage)?;
        Ok(LiveRunSourceContext {
            pins: request.pins(),
            actor_id: source.actor_id,
            request_id: source.request_id,
        })
    }
}
