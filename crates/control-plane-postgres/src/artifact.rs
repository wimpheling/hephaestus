//! Authorized, bounded artifact read application operations.

use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use release_artifact_store::{ArtifactStoreError, LocalArtifactStore};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use std::path::PathBuf;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    sync::mpsc,
};
use uuid::Uuid;

pub const DEFAULT_PREVIEW_BYTES: u32 = 64 * 1024;
pub const MAX_PREVIEW_BYTES: u32 = 1024 * 1024;
pub const DEFAULT_STREAM_TOTAL_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_STREAM_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_STREAM_CHUNK_BYTES: u32 = 64 * 1024;
pub const MAX_STREAM_CHUNK_BYTES: u32 = 256 * 1024;

const CURSOR_DOMAIN: &[u8] = b"hephaestus.artifact-cursor.v1\0";

/// Immutable metadata returned with artifact contents.
#[derive(Debug, Clone)]
pub struct ArtifactMetadata {
    pub id: Uuid,
    pub release_id: Uuid,
    pub build_id: Uuid,
    pub source_commit: String,
    pub path: String,
    pub kind: String,
    pub mode: u32,
    pub sha256: String,
    pub size_bytes: u64,
    pub media_type: String,
}

/// Bounded UTF-8 artifact preview.
pub struct ArtifactPreview {
    pub artifact: ArtifactMetadata,
    pub utf8_contents: String,
    pub truncated: bool,
}

/// Validated artifact stream request.
pub struct StreamArtifact {
    pub artifact_id: Uuid,
    pub resume_cursor: Option<String>,
    pub max_total_bytes: u64,
    pub max_chunk_bytes: u32,
}

/// One committed chunk from the immutable artifact object.
pub struct ArtifactChunk {
    pub sequence: u64,
    pub contents: Vec<u8>,
    pub committed_cursor: String,
    pub end_of_artifact: bool,
    pub media_type: String,
}

/// A cancellation-aware stream receiver for an authorized artifact.
pub struct ArtifactStream {
    pub receiver: mpsc::Receiver<Result<ArtifactChunk, ArtifactError>>,
}

/// Typed artifact application failure.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    /// No authorized artifact row matched the identifier.
    #[error("artifact was not found")]
    NotFound,
    /// A caller-supplied size limit or cursor was invalid.
    #[error("artifact request is invalid")]
    InvalidArgument,
    /// A caller-supplied bound exceeded the service maximum.
    #[error("artifact request exceeds a service bound")]
    ResourceExhausted,
    /// Previewed bytes were not valid UTF-8.
    #[error("artifact preview is not UTF-8")]
    InvalidUtf8,
    /// Persistence failed while evaluating the authorized query.
    #[error("artifact persistence failed")]
    Persistence(#[source] sqlx::Error),
    /// The canonical artifact store rejected the durable object.
    #[error("artifact storage failed")]
    Storage(#[source] ArtifactStoreError),
    /// Reading an already-authorized immutable object failed.
    #[error("artifact read failed")]
    Io(#[source] std::io::Error),
    /// Durable metadata did not match the canonical object.
    #[error("artifact metadata is inconsistent")]
    InvalidStoredData,
}

/// Executes artifact reads after RLS authorization and safe-store resolution.
pub struct ArtifactApplication {
    pool: PgPool,
    store: LocalArtifactStore,
    cursor_key: [u8; 32],
}

impl ArtifactApplication {
    pub const fn new(pool: PgPool, store: LocalArtifactStore, cursor_key: [u8; 32]) -> Self {
        Self {
            pool,
            store,
            cursor_key,
        }
    }

    pub async fn get_artifact_preview(
        &self,
        identity: &AuthenticatedIdentity,
        artifact_id: Uuid,
        max_bytes: u32,
    ) -> Result<ArtifactPreview, ArtifactError> {
        let maximum = if max_bytes == 0 {
            DEFAULT_PREVIEW_BYTES
        } else {
            max_bytes
        };
        if maximum > MAX_PREVIEW_BYTES {
            return Err(ArtifactError::ResourceExhausted);
        }
        let (artifact, path) = self.authorize_artifact(identity, artifact_id).await?;
        let file = open_validated(&path, artifact.size_bytes).await?;
        let capacity = usize::try_from(maximum)
            .map_err(|_| ArtifactError::ResourceExhausted)?
            .saturating_add(1);
        let mut contents = Vec::with_capacity(capacity);
        file.take(u64::from(maximum) + 1)
            .read_to_end(&mut contents)
            .await
            .map_err(ArtifactError::Io)?;
        let truncated = contents.len()
            > usize::try_from(maximum).map_err(|_| ArtifactError::ResourceExhausted)?;
        if truncated {
            contents
                .truncate(usize::try_from(maximum).map_err(|_| ArtifactError::ResourceExhausted)?);
        }
        let utf8_contents = utf8_prefix(contents, truncated)?;
        Ok(ArtifactPreview {
            artifact,
            utf8_contents,
            truncated,
        })
    }

    pub async fn stream_artifact(
        &self,
        identity: &AuthenticatedIdentity,
        request: StreamArtifact,
    ) -> Result<ArtifactStream, ArtifactError> {
        let total_limit = if request.max_total_bytes == 0 {
            DEFAULT_STREAM_TOTAL_BYTES
        } else {
            request.max_total_bytes
        };
        let chunk_limit = if request.max_chunk_bytes == 0 {
            DEFAULT_STREAM_CHUNK_BYTES
        } else {
            request.max_chunk_bytes
        };
        if total_limit > MAX_STREAM_TOTAL_BYTES || chunk_limit > MAX_STREAM_CHUNK_BYTES {
            return Err(ArtifactError::ResourceExhausted);
        }
        if total_limit == 0 || chunk_limit == 0 {
            return Err(ArtifactError::InvalidArgument);
        }
        let (artifact, path) = self
            .authorize_artifact(identity, request.artifact_id)
            .await?;
        let offset = request.resume_cursor.as_deref().map_or(Ok(0), |cursor| {
            decode_cursor(
                cursor,
                &self.cursor_key,
                &identity.user_id.as_uuid(),
                request.artifact_id,
            )
        })?;
        if offset > artifact.size_bytes {
            return Err(ArtifactError::InvalidArgument);
        }
        let mut file = open_validated(&path, artifact.size_bytes).await?;
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(ArtifactError::Io)?;

        let (sender, receiver) = mpsc::channel(2);
        let cursor_key = self.cursor_key;
        let actor_id = identity.user_id.as_uuid();
        let media_type = artifact.media_type.clone();
        let size_bytes = artifact.size_bytes;
        let artifact_id = artifact.id;
        tokio::spawn(async move {
            stream_file(
                file,
                sender,
                StreamState {
                    artifact_id,
                    actor_id,
                    cursor_key,
                    media_type,
                    offset,
                    size_bytes,
                    total_limit,
                    chunk_limit,
                },
            )
            .await;
        });
        Ok(ArtifactStream { receiver })
    }

    async fn authorize_artifact(
        &self,
        identity: &AuthenticatedIdentity,
        artifact_id: Uuid,
    ) -> Result<(ArtifactMetadata, PathBuf), ArtifactError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(ArtifactError::Persistence)?;
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
        transaction
            .commit()
            .await
            .map_err(ArtifactError::Persistence)?;
        let storage_key = row.storage_key;
        let artifact = ArtifactMetadata::try_from(row)?;
        let path = self
            .store
            .resolve(storage_key)
            .map_err(ArtifactError::Storage)?;
        Ok((artifact, path))
    }
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

struct StreamState {
    artifact_id: Uuid,
    actor_id: Uuid,
    cursor_key: [u8; 32],
    media_type: String,
    offset: u64,
    size_bytes: u64,
    total_limit: u64,
    chunk_limit: u32,
}

async fn stream_file(
    mut file: File,
    sender: mpsc::Sender<Result<ArtifactChunk, ArtifactError>>,
    mut state: StreamState,
) {
    let mut sent = 0_u64;
    let mut sequence = 0_u64;
    if state.offset == state.size_bytes {
        let cursor = encode_cursor(
            &state.cursor_key,
            state.actor_id,
            state.artifact_id,
            state.offset,
        );
        let _result = sender
            .send(Ok(ArtifactChunk {
                sequence,
                contents: Vec::new(),
                committed_cursor: cursor,
                end_of_artifact: true,
                media_type: state.media_type,
            }))
            .await;
        return;
    }
    while sent < state.total_limit && state.offset < state.size_bytes {
        let remaining = (state.total_limit - sent).min(state.size_bytes - state.offset);
        let length = remaining.min(u64::from(state.chunk_limit));
        let Ok(length) = usize::try_from(length) else {
            let _result = sender.send(Err(ArtifactError::ResourceExhausted)).await;
            return;
        };
        let mut contents = vec![0_u8; length];
        if let Err(error) = file.read_exact(&mut contents).await {
            let _result = sender.send(Err(ArtifactError::Io(error))).await;
            return;
        }
        let Ok(length) = u64::try_from(contents.len()) else {
            let _result = sender.send(Err(ArtifactError::ResourceExhausted)).await;
            return;
        };
        state.offset += length;
        sent += length;
        let end_of_artifact = state.offset == state.size_bytes;
        let cursor = encode_cursor(
            &state.cursor_key,
            state.actor_id,
            state.artifact_id,
            state.offset,
        );
        if sender
            .send(Ok(ArtifactChunk {
                sequence,
                contents,
                committed_cursor: cursor,
                end_of_artifact,
                media_type: state.media_type.clone(),
            }))
            .await
            .is_err()
        {
            return;
        }
        sequence += 1;
    }
}

async fn open_validated(path: &PathBuf, expected_size: u64) -> Result<File, ArtifactError> {
    let file = File::open(path).await.map_err(ArtifactError::Io)?;
    let metadata = file.metadata().await.map_err(ArtifactError::Io)?;
    if !metadata.is_file() || metadata.len() != expected_size {
        return Err(ArtifactError::InvalidStoredData);
    }
    Ok(file)
}

fn encode_cursor(key: &[u8; 32], actor_id: Uuid, artifact_id: Uuid, offset: u64) -> String {
    let digest = cursor_digest(key, actor_id, artifact_id, offset);
    format!("v1.{offset}.{}", encode_hex(&digest))
}

fn decode_cursor(
    value: &str,
    key: &[u8; 32],
    actor_id: &Uuid,
    artifact_id: Uuid,
) -> Result<u64, ArtifactError> {
    let mut parts = value.split('.');
    if parts.next() != Some("v1") {
        return Err(ArtifactError::InvalidArgument);
    }
    let offset = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or(ArtifactError::InvalidArgument)?;
    let supplied = parts.next().ok_or(ArtifactError::InvalidArgument)?;
    if parts.next().is_some()
        || supplied != encode_hex(&cursor_digest(key, *actor_id, artifact_id, offset))
    {
        return Err(ArtifactError::InvalidArgument);
    }
    Ok(offset)
}

fn cursor_digest(key: &[u8; 32], actor_id: Uuid, artifact_id: Uuid, offset: u64) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(CURSOR_DOMAIN);
    digest.update(key);
    digest.update(actor_id.as_bytes());
    digest.update(artifact_id.as_bytes());
    digest.update(offset.to_be_bytes());
    digest.finalize().into()
}

fn encode_hex(value: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn utf8_prefix(mut contents: Vec<u8>, truncated: bool) -> Result<String, ArtifactError> {
    match std::str::from_utf8(&contents) {
        Ok(_) => String::from_utf8(contents).map_err(|_| ArtifactError::InvalidUtf8),
        Err(error) if truncated && error.error_len().is_none() => {
            contents.truncate(error.valid_up_to());
            String::from_utf8(contents).map_err(|_| ArtifactError::InvalidUtf8)
        }
        Err(_) => Err(ArtifactError::InvalidUtf8),
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_cursor, encode_cursor, utf8_prefix};
    use uuid::Uuid;

    #[test]
    fn resume_cursor_is_bound_to_actor_artifact_and_offset() {
        let key = [7_u8; 32];
        let actor = Uuid::new_v4();
        let artifact = Uuid::new_v4();
        let cursor = encode_cursor(&key, actor, artifact, 42);
        assert!(matches!(
            decode_cursor(&cursor, &key, &actor, artifact),
            Ok(42)
        ));
        assert!(decode_cursor(&cursor, &key, &Uuid::new_v4(), artifact).is_err());
        assert!(decode_cursor(&cursor, &key, &actor, Uuid::new_v4()).is_err());
    }

    #[test]
    fn preview_removes_only_an_incomplete_trailing_codepoint() {
        assert_eq!(
            utf8_prefix(vec![b'a', 0xc3], true).expect("valid bounded prefix"),
            "a"
        );
        assert!(utf8_prefix(vec![b'a', 0xff], true).is_err());
        assert!(utf8_prefix(vec![b'a', 0xc3], false).is_err());
    }
}
