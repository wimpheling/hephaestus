//! Explicit trusted composition; persisted producer data selects each route.

#[path = "invocation/leases.rs"]
mod leases;
#[path = "invocation/prepare.rs"]
mod prepare;
#[path = "invocation/time.rs"]
mod time;

use super::{
    AppError, Duration, EncryptedFileHandoffStore, EncryptedFileRuntimeGitHandoffStore,
    OffsetDateTime, PathBuf, PgPool, PgRunAuthorityManager, PgRuntimeGitCredentialRepository,
    PgRuntimeSessionRepository, Run, RunAuthorityError, RunId, RuntimeCredentialGeneration,
    RuntimeGitCredentialIssuer, RuntimeSessionId, RuntimeSessionIssuer, Uuid, authority_error,
    component, live,
};
use heph_run::{RunKind, RunRepository, RunState};
use release_service::InstanceExecutionConfiguration;
use run_orchestrator::{InvocationCancelEvidence, RepositoryError};
use std::sync::Arc;
use volume_trait::RunVolumeStore;

pub(super) struct QualifiedInvocation {
    configuration: InstanceExecutionConfiguration,
    runs: Arc<dyn RunRepository>,
    volumes: Arc<dyn RunVolumeStore>,
}

impl PgRunAuthorityManager {
    /// Derives both managed Invocation observers from the same trusted worker pool.
    ///
    /// Retains the checked configuration before either issuer is created. Actual
    /// owner/physical validation belongs to the existing managed composition.
    /// No independent Git repository/context can be supplied to this constructor.
    ///
    /// # Errors
    /// Returns the existing qualified constructor's TTL or handoff-storage error.
    pub fn new_managed_invocation(
        pool: PgPool,
        handoff: (PathBuf, [u8; 32]),
        session_ttl: Duration,
        configuration: InstanceExecutionConfiguration,
        runs: Arc<dyn RunRepository>,
        volumes: Arc<dyn RunVolumeStore>,
    ) -> Result<Self, AppError> {
        let git_repository = PgRuntimeGitCredentialRepository::new(pool.clone())
            .with_qualified_invocation_configuration(configuration.clone());
        Self::new_qualified_invocation(
            pool,
            git_repository,
            handoff,
            session_ttl,
            configuration,
            runs,
            volumes,
        )
    }

    /// Creates a worker manager with explicit qualified Invocation composition.
    ///
    /// The handoff tuple is the existing root and host key. Configuration is
    /// checked composition data, not provider/root ownership or caller authority.
    /// Trusted startup must validate managed providers and supply the same exact
    /// volume store and immutable configured Run repository used by execution.
    /// Both resolver and issuer receive the configured repository before cloning.
    /// This constructor enables no eligibility-time issuance or Git for Invocation.
    ///
    /// # Errors
    ///
    /// Rejects a nonpositive/unrepresentable trusted TTL or unavailable encrypted
    /// handoff storage. Per-Run stored lineage and current rights are checked later.
    pub fn new_qualified_invocation(
        pool: PgPool,
        git_repository: PgRuntimeGitCredentialRepository,
        handoff: (PathBuf, [u8; 32]),
        session_ttl: Duration,
        configuration: InstanceExecutionConfiguration,
        runs: Arc<dyn RunRepository>,
        volumes: Arc<dyn RunVolumeStore>,
    ) -> Result<Self, AppError> {
        let session_ttl = ::time::Duration::try_from(session_ttl).map_err(|error| {
            AppError::Configuration(format!("runtime authority session TTL is invalid: {error}"))
        })?;
        if session_ttl <= ::time::Duration::ZERO {
            return Err(AppError::Configuration(
                "qualified runtime authority session TTL must be positive".to_owned(),
            ));
        }
        let repository = PgRuntimeSessionRepository::new(pool)
            .with_canonical_live_snapshots()
            .with_qualified_invocation_configuration(configuration.clone());
        let (handoff_root, handoff_key) = handoff;
        let handoff = EncryptedFileHandoffStore::new(handoff_root.clone(), handoff_key)
            .map_err(component("runtime authority handoff"))?;
        let git_handoff = EncryptedFileRuntimeGitHandoffStore::new(handoff_root, handoff_key)
            .map_err(component("runtime Git authority handoff"))?;
        Ok(Self {
            issuer: RuntimeSessionIssuer::new(repository.clone(), handoff),
            git_issuer: RuntimeGitCredentialIssuer::new(git_repository, git_handoff),
            repository,
            session_ttl,
            canonical_live_authority: true,
            invocation: Some(QualifiedInvocation {
                configuration,
                runs,
                volumes,
            }),
        })
    }

    pub(super) async fn invocation_run(
        &self,
        supplied: &Run,
    ) -> Result<Option<Run>, RunAuthorityError> {
        let Some(qualified) = &self.invocation else {
            return if supplied.kind == RunKind::Invocation {
                Err(denied("qualified Invocation authority is unsupported"))
            } else {
                Ok(None)
            };
        };
        let current = qualified
            .runs
            .get(supplied.id)
            .await
            .map_err(|error| denied(error.to_string()))?;
        if current.kind == RunKind::Invocation || supplied.kind == RunKind::Invocation {
            if &current != supplied || current.kind != RunKind::Invocation {
                return Err(denied("Invocation differs from the exact persisted Run"));
            }
            return Ok(Some(current));
        }
        Ok(None)
    }

    pub(super) async fn reauthorize_invocation(&self, run: &Run) -> Result<(), RunAuthorityError> {
        if run.state == RunState::LeasingVolume {
            return self
                .repository
                .verify_qualified_invocation_pre_acquisition(
                    run,
                    authz_postgres::AUTHORIZATION_MODEL_VERSION,
                )
                .await
                .map_err(authority_error);
        }
        live::snapshot(&self.repository, run).await.map(|_| ())
    }

    pub(super) async fn acknowledge_invocation(
        &self,
        run: &Run,
        session_id: Uuid,
        generation: u64,
    ) -> Result<(), RunAuthorityError> {
        if run.state != RunState::Provisioning
            || session_id != run.id.as_uuid()
            || generation != RuntimeCredentialGeneration::INITIAL.get()
        {
            return Err(denied(
                "Invocation acknowledgement differs from its exact bootstrap",
            ));
        }
        self.issuer
            .acknowledge(
                RuntimeSessionId::from_uuid(session_id),
                RuntimeCredentialGeneration::INITIAL,
                OffsetDateTime::now_utc(),
            )
            .await
            .map_err(authority_error)?;
        Ok(())
    }

    pub(super) async fn invocation_cleanup_run(
        &self,
        run: RunId,
    ) -> Result<bool, RunAuthorityError> {
        let Some(qualified) = &self.invocation else {
            return Ok(false);
        };
        let current = qualified
            .runs
            .get(run)
            .await
            .map_err(|error| denied(error.to_string()))?;
        Ok(current.kind == RunKind::Invocation)
    }

    pub(super) async fn revoke_invocation(&self, run: RunId) -> Result<(), RunAuthorityError> {
        // Do not pre-find a withdrawn pending session: B1 pending replay checks
        // intentionally reject it. Exact metadata revocation precedes historical
        // find, which permits its now-revoked graph without live execution rights.
        // Keep the nonExpired revocation path unchanged. Only its exact Expired
        // sentinel can attempt fully qualified expired observation; a missing or
        // withdrawn Pending graph is never inferred to be an expired original.
        let session = RuntimeSessionId::from_uuid(run.as_uuid());
        match self
            .issuer
            .revoke(session, OffsetDateTime::now_utc(), "run guest destroyed")
            .await
        {
            Ok(_) => Ok(()),
            Err(runtime_authority::RuntimeAuthorityError::NotFound) => {
                let qualified = self.invocation.as_ref().ok_or_else(|| {
                    denied("qualified Invocation cleanup configuration is absent")
                })?;
                // Absence alone is not cleanup authority. Requalify the genuine
                // queued cancellation, full zero history and prepared target
                // through the SAME configured worker repository and owner.
                queued_authority_absence(
                    qualified
                        .runs
                        .classify_invocation_cancel(run, qualified.configuration.provider_scope())
                        .await,
                )
            }
            Err(runtime_authority::RuntimeAuthorityError::SessionNotPending) => {
                super::cleanup::expired(
                    &self.issuer,
                    &self.git_issuer,
                    run.as_uuid(),
                    session,
                    RuntimeCredentialGeneration::INITIAL,
                )
                .await
            }
            Err(error) => Err(authority_error(error)),
        }
    }
}

fn denied(message: impl Into<String>) -> RunAuthorityError {
    RunAuthorityError::redacted(message)
}

// This mapper receives positive repository evidence, never an absent session
// lookup. Unknown/foreign/unsupported history remains the original failure.
fn queued_authority_absence(
    evidence: Result<InvocationCancelEvidence, RepositoryError>,
) -> Result<(), RunAuthorityError> {
    match evidence {
        Ok(InvocationCancelEvidence::Queued115) => Ok(()),
        Ok(InvocationCancelEvidence::Consumed117) => Err(authority_error(
            runtime_authority::RuntimeAuthorityError::NotFound,
        )),
        Err(error) => Err(denied(error.to_string())),
    }
}

#[cfg(test)]
#[path = "invocation/cleanup_tests.rs"]
mod cleanup_tests;
