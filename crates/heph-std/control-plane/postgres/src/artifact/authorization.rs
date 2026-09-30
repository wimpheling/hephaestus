use super::{ArtifactApplication, ArtifactError, ArtifactMetadata};
use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{audit_decision, begin_actor_transaction_on_connection};
use identity_domain::AuthenticatedIdentity;
use serde_json::Value;
use sqlx::{
    Connection, FromRow, PgPool, Postgres,
    pool::PoolConnection,
    postgres::{PgConnectOptions, PgConnection},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    },
    time::Duration,
};
use tokio::time::timeout;
use uuid::Uuid;

const AUTHORIZATION_CANCEL_TIMEOUT: Duration = Duration::from_secs(2);

/// Keeps a checked-out connection from returning to the pool while a query is
/// being canceled. Successful authorization explicitly releases the guard.
struct AuthorizationConnection {
    connection: Option<PoolConnection<Postgres>>,
    connect_options: Arc<PgConnectOptions>,
    backend_pid: Arc<AtomicI32>,
}

impl AuthorizationConnection {
    async fn acquire(pool: &PgPool) -> Result<Self, sqlx::Error> {
        Ok(Self {
            connection: Some(pool.acquire().await?),
            connect_options: pool.connect_options(),
            backend_pid: Arc::new(AtomicI32::new(0)),
        })
    }

    const fn connection_mut(&mut self) -> &mut PoolConnection<Postgres> {
        self.connection
            .as_mut()
            .expect("authorization connection remains owned")
    }

    fn release(mut self) {
        self.connection.take();
        self.backend_pid.store(0, Ordering::Relaxed);
    }
}

impl Drop for AuthorizationConnection {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.as_mut() {
            connection.close_on_drop();
            let backend_pid = self.backend_pid.load(Ordering::Relaxed);
            if backend_pid > 0 {
                let connect_options = Arc::clone(&self.connect_options);
                let cancellation = async move {
                    let result = timeout(
                        AUTHORIZATION_CANCEL_TIMEOUT,
                        cancel_backend_query(connect_options, backend_pid),
                    )
                    .await;
                    match result {
                        Ok(Ok(true)) => tracing::debug!(
                            backend_pid,
                            "canceled dropped artifact authorization query"
                        ),
                        Ok(Ok(false)) => tracing::debug!(
                            backend_pid,
                            "artifact authorization query was already inactive"
                        ),
                        Ok(Err(_)) => tracing::warn!(
                            backend_pid,
                            "failed to cancel dropped artifact authorization query"
                        ),
                        Err(_) => tracing::warn!(
                            backend_pid,
                            "timed out canceling dropped artifact authorization query"
                        ),
                    }
                };
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(cancellation);
                } else {
                    tracing::warn!(
                        backend_pid,
                        "could not schedule artifact authorization query cancellation"
                    );
                }
            }
        }
    }
}

// This operational query targets the checked-out backend after the actor-scoped
// authorization future is dropped, so it intentionally runs outside that transaction.
async fn cancel_backend_query(
    connect_options: Arc<PgConnectOptions>,
    backend_pid: i32,
) -> Result<bool, sqlx::Error> {
    let mut connection = PgConnection::connect_with(&connect_options).await?;
    let canceled: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)")
        .bind(backend_pid)
        .fetch_one(&mut connection)
        .await?;
    connection.close().await?;
    Ok(canceled)
}

pub(super) async fn authorize_artifact(
    application: &ArtifactApplication,
    identity: &AuthenticatedIdentity,
    artifact_id: Uuid,
) -> Result<(ArtifactMetadata, std::path::PathBuf), ArtifactError> {
    let mut connection = AuthorizationConnection::acquire(&application.pool)
        .await
        .map_err(ArtifactError::Persistence)?;
    let backend_pid_slot = Arc::clone(&connection.backend_pid);
    let mut transaction =
        begin_actor_transaction_on_connection(connection.connection_mut(), identity)
            .await
            .map_err(ArtifactError::Persistence)?;
    let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *transaction)
        .await
        .map_err(ArtifactError::Persistence)?;
    backend_pid_slot.store(backend_pid, Ordering::Relaxed);
    let row = sqlx::query_as::<_, ArtifactRow>(
        "SELECT artifact.id, artifact.release_id, release.build_request_id,
                release.source_commit, artifact.path, artifact.kind,
                artifact.mode, encode(artifact.content_hash, 'hex') AS sha256,
                artifact.size_bytes, artifact.media_type, artifact.storage_key,
                artifact.provenance
         FROM release_artifacts artifact
         JOIN releases release ON release.id = artifact.release_id
         WHERE artifact.id = $1",
    )
    .bind(artifact_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(ArtifactError::Persistence)?
    .ok_or(ArtifactError::NotFound)?;
    let object = ObjectRef::new(ObjectType::Release, row.release_id);
    let decision = application
        .authorizer
        .check(
            &mut transaction,
            Subject::User(identity.user_id),
            Permission::CanRead,
            object,
        )
        .await
        .map_err(ArtifactError::Authorization)?;
    audit_decision(
        &mut transaction,
        identity.user_id,
        Permission::CanRead,
        object,
        decision,
        identity.request_id,
    )
    .await
    .map_err(ArtifactError::Persistence)?;
    transaction
        .commit()
        .await
        .map_err(ArtifactError::Persistence)?;
    connection.release();
    if !decision.is_allowed() {
        return Err(ArtifactError::NotFound);
    }
    let storage_key = row.storage_key;
    let artifact = ArtifactMetadata::try_from(row)?;
    let path = application
        .store
        .resolve(storage_key)
        .map_err(ArtifactError::Storage)?;
    Ok((artifact, path))
}

#[derive(FromRow)]
struct ArtifactRow {
    id: Uuid,
    release_id: Uuid,
    build_request_id: Uuid,
    source_commit: String,
    path: String,
    kind: String,
    mode: i32,
    sha256: String,
    size_bytes: i64,
    media_type: String,
    storage_key: Uuid,
    #[allow(dead_code)]
    provenance: Value,
}

impl TryFrom<ArtifactRow> for ArtifactMetadata {
    type Error = ArtifactError;

    fn try_from(row: ArtifactRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            release_id: row.release_id,
            build_id: row.build_request_id,
            source_commit: row.source_commit,
            path: row.path,
            kind: row.kind,
            mode: u32::try_from(row.mode).map_err(|_| ArtifactError::InvalidStoredData)?,
            sha256: row.sha256,
            size_bytes: u64::try_from(row.size_bytes)
                .map_err(|_| ArtifactError::InvalidStoredData)?,
            media_type: row.media_type,
        })
    }
}
