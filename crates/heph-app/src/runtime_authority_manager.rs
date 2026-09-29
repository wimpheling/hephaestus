use super::{
    AppError, Duration, EncryptedFileHandoffStore, OffsetDateTime, PathBuf, PgPool, Run, RunId,
    Uuid, component,
};
use async_trait::async_trait;
use capability_domain::{
    RuntimeCredentialGeneration, RuntimeInvocation, RuntimeSessionId, RuntimeSessionIdentity,
};
use heph_run::{PreparedRunAuthority, RunAuthorityError, RunAuthorityManager};
use runtime_authority::{RuntimeSessionIssuer, RuntimeSessionRepository};
use runtime_authority_postgres::PgRuntimeSessionRepository;
use runtime_git_authority::{RuntimeGitAuthorityError, RuntimeGitCredentialIssuer};
use runtime_git_authority_postgres::PgRuntimeGitCredentialRepository;
use runtime_handoff_local::EncryptedFileRuntimeGitHandoffStore;
/// Persisted run lifecycle event used by operational waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEventKind {
    /// The VM reached its running state.
    Running,
    /// The trusted host completed controlled result publication.
    ResultCompleted,
}

impl RunEventKind {
    /// Returns the stable event name used by waits and durable event rows.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "run.running",
            Self::ResultCompleted => "result.completed",
        }
    }
}

pub struct PgRunAuthorityManager {
    repository: PgRuntimeSessionRepository,
    issuer: RuntimeSessionIssuer<PgRuntimeSessionRepository, EncryptedFileHandoffStore>,
    git_issuer: RuntimeGitCredentialIssuer<
        PgRuntimeGitCredentialRepository,
        EncryptedFileRuntimeGitHandoffStore,
    >,
    session_ttl: time::Duration,
}

impl PgRunAuthorityManager {
    pub fn new(
        pool: PgPool,
        git_repository: PgRuntimeGitCredentialRepository,
        handoff_root: PathBuf,
        handoff_key: [u8; 32],
        session_ttl: Duration,
    ) -> Result<Self, AppError> {
        let repository = PgRuntimeSessionRepository::new(pool);
        let handoff = EncryptedFileHandoffStore::new(handoff_root.clone(), handoff_key)
            .map_err(component("runtime authority handoff"))?;
        let git_handoff = EncryptedFileRuntimeGitHandoffStore::new(handoff_root, handoff_key)
            .map_err(component("runtime Git authority handoff"))?;
        let session_ttl = time::Duration::try_from(session_ttl).map_err(|error| {
            AppError::Configuration(format!("runtime authority session TTL is invalid: {error}"))
        })?;
        Ok(Self {
            issuer: RuntimeSessionIssuer::new(repository.clone(), handoff),
            git_issuer: RuntimeGitCredentialIssuer::new(git_repository, git_handoff),
            repository,
            session_ttl,
        })
    }

    async fn snapshot(
        &self,
        run: &Run,
    ) -> Result<capability_domain::AuthorizationSnapshot, RunAuthorityError> {
        self.repository
            .resolve_snapshot(run, authz_postgres::AUTHORIZATION_MODEL_VERSION)
            .await
            .map_err(authority_error)
    }
}

#[async_trait]
impl RunAuthorityManager for PgRunAuthorityManager {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunAuthority, RunAuthorityError> {
        let snapshot = self.snapshot(run).await?;
        if !self
            .repository
            .live_authorized(run, &snapshot)
            .await
            .map_err(authority_error)?
        {
            return Err(RunAuthorityError::redacted(
                "live capability authority was denied",
            ));
        }
        let session_id = RuntimeSessionId::from_uuid(run.id.as_uuid());
        let existing = self
            .repository
            .find(session_id)
            .await
            .map_err(authority_error)?;
        let issued_at = existing
            .as_ref()
            .map_or_else(OffsetDateTime::now_utc, |session| session.issued_at);
        let expires_at = existing
            .as_ref()
            .map_or(issued_at + self.session_ttl, |session| session.expires_at);
        let identity = RuntimeSessionIdentity::new(
            session_id,
            snapshot.principal(),
            RuntimeInvocation::Run(run.id),
            &snapshot,
            issued_at,
            expires_at,
        )
        .map_err(|_| RunAuthorityError::redacted("runtime identity is invalid"))?;
        let issued = self
            .issuer
            .issue(
                &snapshot,
                &identity,
                run.attachment_id
                    .map(runtime_types::AgentAttachmentId::as_uuid),
                OffsetDateTime::now_utc(),
            )
            .await
            .map_err(authority_error)?;
        let runtime_git = match self
            .git_issuer
            .issue(
                issued.session.id,
                issued.session.generation,
                issued.session.expires_at,
                OffsetDateTime::now_utc(),
            )
            .await
        {
            Ok(issued) => Some(issued),
            Err(RuntimeGitAuthorityError::NotFound) => None,
            Err(error) => return Err(runtime_git_authority_error(error)),
        };
        let mut bootstrap = heph_runtime::RuntimeAuthorityBootstrap::new(
            issued.session.id.as_uuid(),
            issued.session.generation.get(),
            *issued.credential.expose(),
        );
        if let Some(runtime_git) = &runtime_git {
            bootstrap = bootstrap.with_runtime_git_credential(*runtime_git.credential.expose());
        }
        Ok(PreparedRunAuthority {
            bootstrap: Some(bootstrap),
        })
    }

    async fn reauthorize(&self, run: &Run) -> Result<(), RunAuthorityError> {
        let snapshot = self.snapshot(run).await?;
        if self
            .repository
            .live_authorized(run, &snapshot)
            .await
            .map_err(authority_error)?
        {
            Ok(())
        } else {
            Err(RunAuthorityError::redacted(
                "live capability authority was revoked",
            ))
        }
    }

    async fn acknowledge(
        &self,
        run: &Run,
        session_id: Uuid,
        generation: u64,
    ) -> Result<(), RunAuthorityError> {
        if session_id != run.id.as_uuid() {
            return Err(RunAuthorityError::redacted(
                "runtime session does not match the exact run",
            ));
        }
        let generation = RuntimeCredentialGeneration::new(generation)
            .map_err(|_| RunAuthorityError::redacted("runtime generation is invalid"))?;
        self.issuer
            .acknowledge(
                RuntimeSessionId::from_uuid(session_id),
                generation,
                OffsetDateTime::now_utc(),
            )
            .await
            .map_err(authority_error)?;
        self.git_issuer
            .acknowledge_or_revoke(RuntimeSessionId::from_uuid(session_id), generation)
            .map_err(runtime_git_authority_error)?;
        Ok(())
    }

    async fn revoke_after_guest(&self, run_id: RunId) -> Result<(), RunAuthorityError> {
        let session_id = RuntimeSessionId::from_uuid(run_id.as_uuid());
        let Some(session) = self
            .repository
            .find(session_id)
            .await
            .map_err(authority_error)?
        else {
            return Ok(());
        };
        if matches!(
            session.status,
            capability_domain::RuntimeSessionStatus::Expired
        ) {
            self.issuer
                .recover_expired(OffsetDateTime::now_utc())
                .await
                .map_err(authority_error)?;
            self.git_issuer
                .recover_expired(OffsetDateTime::now_utc())
                .map_err(runtime_git_authority_error)?;
            return Ok(());
        }
        self.issuer
            .revoke(session_id, OffsetDateTime::now_utc(), "run guest destroyed")
            .await
            .map_err(authority_error)?;
        self.git_issuer
            .acknowledge_or_revoke(session_id, session.generation)
            .map_err(runtime_git_authority_error)?;
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunAuthorityError> {
        let recovered = self
            .issuer
            .recover_expired(OffsetDateTime::now_utc())
            .await
            .map_err(authority_error)?;
        let git_recovered = self
            .git_issuer
            .recover_expired(OffsetDateTime::now_utc())
            .map_err(runtime_git_authority_error)?;
        usize::try_from(recovered.max(git_recovered))
            .map_err(|_| RunAuthorityError::redacted("recovery count overflowed"))
    }
}

fn authority_error(error: runtime_authority::RuntimeAuthorityError) -> RunAuthorityError {
    RunAuthorityError::redacted(error.to_string())
}

fn runtime_git_authority_error(error: RuntimeGitAuthorityError) -> RunAuthorityError {
    RunAuthorityError::redacted(error.to_string())
}
