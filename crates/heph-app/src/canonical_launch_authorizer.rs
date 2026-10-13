//! Explicit owned launch checks; caller rights are separate from mount grants.

use std::sync::Arc;

use async_trait::async_trait;
use control_plane_postgres::launch::PgRunLaunchAuthorizer;
use heph_run::Run;
use identity_domain::{RequestId, UserId};
use run_orchestrator::{RunAuthorizationError, RunLaunchAuthorizer};
use runtime_authority::LiveRunSnapshotResolver;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use time::OffsetDateTime;

use crate::runtime_authority_manager::live;

#[path = "canonical_launch_authorizer/invocation.rs"]
mod invocation;

/// Trusted app wrapper for canonical checks with legacy scalar delegation.
///
/// The supplied repository must explicitly enable canonical live resolution.
/// Construction does not enable dispatch or establish a producer principal.
pub struct CanonicalRunLaunchAuthorizer {
    repository: PgRuntimeSessionRepository,
    scalar: Arc<PgRunLaunchAuthorizer>,
    invocation_runs: Option<Arc<dyn heph_run::RunRepository>>,
}

impl CanonicalRunLaunchAuthorizer {
    /// Combines explicit canonical source checks with the existing scalar port.
    #[must_use]
    pub const fn new(
        repository: PgRuntimeSessionRepository,
        scalar: Arc<PgRunLaunchAuthorizer>,
    ) -> Self {
        Self {
            repository,
            scalar,
            invocation_runs: None,
        }
    }
}

#[async_trait]
impl RunLaunchAuthorizer for CanonicalRunLaunchAuthorizer {
    async fn authorize(&self, run: &Run) -> Result<(), RunAuthorizationError> {
        if let Some(current) = self.invocation_run(run).await? {
            return invocation::authorize(&self.repository, &current).await;
        }
        self.scalar.authorize(run).await
    }

    async fn authorize_with_volumes(&self, run: &Run) -> Result<(), RunAuthorizationError> {
        if let Some(current) = self.invocation_run(run).await? {
            return invocation::authorize(&self.repository, &current).await;
        }
        // This port has no cardinality argument and runs even for an empty set.
        // DB-loaded session facts select PreIssuance or Issued; neither this
        // request helper nor any following read creates runtime authority.
        let request = live::request_for_run(&self.repository, run)
            .await
            .map_err(error)?;
        self.repository
            .resolve_live_snapshot(
                &request,
                authz_postgres::AUTHORIZATION_MODEL_VERSION,
                OffsetDateTime::now_utc(),
            )
            .await
            .map_err(error)?;
        let source = self
            .repository
            .recorded_live_source_context(&request)
            .await
            .map_err(error)?;
        self.scalar
            .authorize_canonical_source(
                run,
                UserId::from_uuid(source.actor_id()),
                RequestId::from_uuid(source.request_id()),
                request.ceiling().is_some(),
            )
            .await
    }
}

fn error(error: impl std::fmt::Display) -> RunAuthorizationError {
    // Errors from this trusted adapter contain no request-only plaintext.
    tracing::warn!(%error, "canonical launch authority denied");
    RunAuthorizationError::redacted("canonical caller/source authority unavailable")
}

#[cfg(test)]
#[path = "canonical_launch_authorizer/postgres.rs"]
mod postgres;
