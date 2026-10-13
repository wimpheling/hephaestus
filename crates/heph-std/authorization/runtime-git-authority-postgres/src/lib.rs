//! `PostgreSQL` persistence and live authentication for runtime Git bearers.

mod expired;
mod rows;
use rows::{AuthenticatedRow, StoredRow, parse_operations, permission, persisted_hash};

use async_trait::async_trait;
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
use git_capability_domain::{GitOperation, RepositoryId};
use runtime_git_authority::{
    AuthenticatedRuntimeGitAuthority, RuntimeGitAuthorityError, RuntimeGitCredentialHash,
    RuntimeGitCredentialRepository, StoredRuntimeGitCredential,
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// `PostgreSQL` runtime Git credential adapter.
#[derive(Clone)]
pub struct PgRuntimeGitCredentialRepository {
    pool: PgPool,
    expired_original: Option<runtime_authority_postgres::PgRuntimeSessionRepository>,
}

impl PgRuntimeGitCredentialRepository {
    /// Creates an adapter using a control-plane connection pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            expired_original: None,
        }
    }

    /// Retains the trusted Legacy scope for exact expired observation only.
    ///
    /// The observer is derived from this adapter's original pool. This selects
    /// no live profile, issuance, provider ownership or dispatch authority.
    #[must_use]
    pub fn with_expired_legacy_vm_placement_scope(
        mut self,
        scope: run_domain::LegacyVmPlacementScope,
    ) -> Self {
        let observer = self.expired_original.take().unwrap_or_else(|| {
            runtime_authority_postgres::PgRuntimeSessionRepository::new(self.pool.clone())
        });
        self.expired_original = Some(observer.with_expired_legacy_vm_placement_scope(scope));
        self
    }

    /// Retains checked Invocation configuration for expired observation only.
    ///
    /// Actual provider ownership is qualified by trusted startup separately.
    /// No independent repository or pool can be supplied for this operation.
    #[must_use]
    pub fn with_qualified_invocation_configuration(
        mut self,
        configuration: release_service::InstanceExecutionConfiguration,
    ) -> Self {
        let observer = self.expired_original.take().unwrap_or_else(|| {
            runtime_authority_postgres::PgRuntimeSessionRepository::new(self.pool.clone())
        });
        self.expired_original =
            Some(observer.with_qualified_invocation_configuration(configuration));
        self
    }
}

#[async_trait]
impl RuntimeGitCredentialRepository for PgRuntimeGitCredentialRepository {
    async fn find(
        &self,
        session_id: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeGitCredential>, RuntimeGitAuthorityError> {
        sqlx::query_as::<_, StoredRow>(
            "SELECT runtime_session_id, authorization_snapshot_id, binding_id,
                    repository_id, scope_hash, issuance_generation, expires_at
             FROM runtime_git_credentials WHERE runtime_session_id = $1",
        )
        .bind(session_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn create(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        credential_hash: RuntimeGitCredentialHash,
    ) -> Result<StoredRuntimeGitCredential, RuntimeGitAuthorityError> {
        let generation = i64::try_from(generation.get()).map_err(storage)?;
        sqlx::query_as::<_, StoredRow>(
            "INSERT INTO runtime_git_credentials
                (runtime_session_id, authorization_snapshot_id, binding_id,
                 repository_id, scope_hash, issuance_generation,
                 credential_hash, expires_at)
             SELECT session.id, session.snapshot_id, git.binding_id,
                    git.repository_id, git.normalized_hash,
                    session.issuance_generation, $3, session.expires_at
             FROM runtime_authority_sessions AS session
             JOIN run_git_authority_snapshots AS git
               ON git.snapshot_id = session.snapshot_id
             WHERE session.id = $1
               AND session.issuance_generation = $2
               AND session.status = 'pending_handoff'
               AND session.expires_at > now()
             RETURNING runtime_session_id, authorization_snapshot_id,
                       binding_id, repository_id, scope_hash,
                       issuance_generation, expires_at",
        )
        .bind(session_id.as_uuid())
        .bind(generation)
        .bind(credential_hash.as_bytes().as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(RuntimeGitAuthorityError::NotFound)?
        .try_into()
    }

    async fn observe_expired_run_handoff(
        &self,
        run: Uuid,
        session: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<runtime_git_authority::ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError> {
        self.observe_expired_original(run, session, generation)
            .await
    }

    async fn authenticate(
        &self,
        credential_hash: RuntimeGitCredentialHash,
        repository_id: RepositoryId,
        operation: GitOperation,
        _evaluated_at: OffsetDateTime,
    ) -> Result<AuthenticatedRuntimeGitAuthority, RuntimeGitAuthorityError> {
        let row = sqlx::query_as::<_, AuthenticatedRow>(
            "SELECT * FROM authenticate_runtime_git_credential($1, $2, $3)",
        )
        .bind(credential_hash.as_bytes().as_slice())
        .bind(repository_id.as_uuid())
        .bind(operation_name(operation))
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(RuntimeGitAuthorityError::NotFound)?;
        row.try_into()
    }
}

const fn operation_name(operation: GitOperation) -> &'static str {
    match operation {
        GitOperation::Discover => "discover",
        GitOperation::Fetch => "fetch",
        GitOperation::Receive => "receive",
    }
}

fn storage(_error: impl std::fmt::Display) -> RuntimeGitAuthorityError {
    RuntimeGitAuthorityError::Persistence
}
