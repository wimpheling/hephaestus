//! Persisted Invocation launch checks; no scalar source identity or issuance.

use heph_run::{RunKind, RunRepository, RunState};
use release_service::InstanceExecutionConfiguration;

use super::{
    Arc, CanonicalRunLaunchAuthorizer, PgRunLaunchAuthorizer, PgRuntimeSessionRepository, Run,
    RunAuthorizationError, error, live,
};

impl CanonicalRunLaunchAuthorizer {
    /// Combines qualified worker proofs with existing ordinary launch checks.
    ///
    /// The checked configuration is composition data, not provider/root ownership
    /// or caller authority. Trusted startup must validate managed providers and
    /// route protected producers, and supply the same configured immutable Run
    /// repository used by execution. Those composition prerequisites remain.
    /// Requires the qualified schema and worker repository. This enables no
    /// issuance, credential, scalar fallback or execution IO for Invocation.
    #[must_use]
    pub fn new_qualified_invocation(
        repository: PgRuntimeSessionRepository,
        scalar: Arc<PgRunLaunchAuthorizer>,
        configuration: InstanceExecutionConfiguration,
        runs: Arc<dyn RunRepository>,
    ) -> Self {
        Self {
            repository: repository
                .with_canonical_live_snapshots()
                .with_qualified_invocation_configuration(configuration),
            scalar,
            invocation_runs: Some(runs),
        }
    }

    pub(super) async fn invocation_run(
        &self,
        supplied: &Run,
    ) -> Result<Option<Run>, RunAuthorizationError> {
        let Some(runs) = &self.invocation_runs else {
            return if supplied.kind == RunKind::Invocation {
                Err(error("qualified Invocation launch is unsupported"))
            } else {
                Ok(None)
            };
        };
        // Read before testing kind or reaching an ordinary/scalar method. Stored
        // actor/request IDs are comparison data, never an authenticated identity.
        let current = runs.get(supplied.id).await.map_err(error)?;
        if current.kind == RunKind::Invocation || supplied.kind == RunKind::Invocation {
            if &current != supplied || current.kind != RunKind::Invocation {
                return Err(error("Invocation differs from the exact persisted Run"));
            }
            return Ok(Some(current));
        }
        Ok(None)
    }
}

pub(super) async fn authorize(
    repository: &PgRuntimeSessionRepository,
    run: &Run,
) -> Result<(), RunAuthorizationError> {
    if run.state == RunState::LeasingVolume {
        // Eligibility only: exact fresh115/117/config/current rights/full mounts
        // and no own history. No snapshot/session/credential/IO authority follows.
        return repository
            .verify_qualified_invocation_pre_acquisition(
                run,
                authz_postgres::AUTHORIZATION_MODEL_VERSION,
            )
            .await
            .map_err(error);
    }
    // B1 retains its full live phase/ceiling/ACK/attached-set meanings. Never
    // delegate this checked Invocation to the scalar source/identity path.
    live::snapshot(repository, run)
        .await
        .map(|_| ())
        .map_err(error)
}
