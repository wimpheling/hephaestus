//! Authorized, bounded artifact read application operations.

use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::AuthenticatedIdentity;
use release_artifact_store::LocalArtifactStore;
use sqlx::PgPool;
use std::path::PathBuf;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
};
use uuid::Uuid;

#[path = "artifact/authorization.rs"]
mod authorization;
#[path = "artifact/cursor.rs"]
mod cursor;
#[path = "artifact/model.rs"]
mod model;
#[path = "artifact/stream.rs"]
mod stream;
use authorization::authorize_artifact;
use cursor::decode_cursor;
pub use model::{ArtifactError, ArtifactMetadata, ArtifactPreview};
pub use stream::{ArtifactCancellation, ArtifactChunk, ArtifactStream, StreamArtifact};

pub const DEFAULT_PREVIEW_BYTES: u32 = 64 * 1024;
pub const MAX_PREVIEW_BYTES: u32 = 1024 * 1024;
pub const DEFAULT_STREAM_TOTAL_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_STREAM_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_STREAM_CHUNK_BYTES: u32 = 64 * 1024;
pub const MAX_STREAM_CHUNK_BYTES: u32 = 256 * 1024;

/// Executes artifact reads after RLS authorization and safe-store resolution.
pub struct ArtifactApplication {
    pool: PgPool,
    store: LocalArtifactStore,
    cursor_key: [u8; 32],
    authorizer: PostgresMelangeAuthorizer,
}

impl ArtifactApplication {
    pub const fn new(pool: PgPool, store: LocalArtifactStore, cursor_key: [u8; 32]) -> Self {
        Self {
            pool,
            store,
            cursor_key,
            authorizer: PostgresMelangeAuthorizer,
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
        let (artifact, path) = authorize_artifact(self, identity, artifact_id).await?;
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
        self.stream_artifact_with_budget(identity, request, stream::NoCancellation, None)
            .await
    }

    /// Starts an artifact stream with a transport cancellation and deadline.
    pub async fn stream_artifact_with_budget<Cancellation>(
        &self,
        identity: &AuthenticatedIdentity,
        request: StreamArtifact,
        cancellation: Cancellation,
        deadline: Option<std::time::Instant>,
    ) -> Result<ArtifactStream, ArtifactError>
    where
        Cancellation: ArtifactCancellation,
    {
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
        let (artifact, path) = authorize_artifact(self, identity, request.artifact_id).await?;
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

        Ok(stream::spawn_reader(
            file,
            stream::StreamState {
                artifact_id: artifact.id,
                actor_id: identity.user_id.as_uuid(),
                cursor_key: self.cursor_key,
                media_type: artifact.media_type,
                offset,
                size_bytes: artifact.size_bytes,
                total_limit,
                chunk_limit,
            },
            cancellation,
            deadline,
        ))
    }
}

pub(super) async fn open_validated(
    path: &PathBuf,
    expected_size: u64,
) -> Result<File, ArtifactError> {
    let file = File::open(path).await.map_err(ArtifactError::Io)?;
    let metadata = file.metadata().await.map_err(ArtifactError::Io)?;
    if !metadata.is_file() || metadata.len() != expected_size {
        return Err(ArtifactError::InvalidStoredData);
    }
    Ok(file)
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
    use super::{cursor::encode_cursor, decode_cursor, utf8_prefix};
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
