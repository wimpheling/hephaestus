use super::{ArtifactError, cursor::encode_cursor};
use std::{future::Future, pin::Pin, time::Instant};
use tokio::{fs::File, io::AsyncReadExt, sync::mpsc};
use uuid::Uuid;

/// Cancellation contract shared by transport adapters and artifact readers.
pub trait ArtifactCancellation: Clone + Send + Sync + 'static {
    fn is_cancelled(&self) -> bool;
    fn cancel(&self);
    fn cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

#[derive(Clone, Copy)]
pub struct NoCancellation;

impl ArtifactCancellation for NoCancellation {
    fn is_cancelled(&self) -> bool {
        false
    }

    fn cancel(&self) {}

    fn cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(std::future::pending())
    }
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

pub(super) struct StreamState {
    pub(super) artifact_id: Uuid,
    pub(super) actor_id: Uuid,
    pub(super) cursor_key: [u8; 32],
    pub(super) media_type: String,
    pub(super) offset: u64,
    pub(super) size_bytes: u64,
    pub(super) total_limit: u64,
    pub(super) chunk_limit: u32,
}

pub(super) fn spawn_reader<Cancellation>(
    file: File,
    state: StreamState,
    cancellation: Cancellation,
    deadline: Option<Instant>,
) -> ArtifactStream
where
    Cancellation: ArtifactCancellation,
{
    let (sender, receiver) = mpsc::channel(2);
    tokio::spawn(stream_file(file, sender, state, cancellation, deadline));
    ArtifactStream { receiver }
}

#[derive(Debug, PartialEq, Eq)]
enum Boundary {
    Closed,
    Canceled,
    Deadline,
}

async fn run_with_boundary<Cancellation, Operation, Output>(
    cancellation: &Cancellation,
    sender: &mpsc::Sender<Result<ArtifactChunk, ArtifactError>>,
    deadline: Option<Instant>,
    operation: Operation,
) -> Result<Output, Boundary>
where
    Cancellation: ArtifactCancellation,
    Operation: Future<Output = Output>,
{
    if cancellation.is_cancelled() {
        return Err(Boundary::Canceled);
    }
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        cancellation.cancel();
        return Err(Boundary::Deadline);
    }
    match deadline {
        Some(deadline) => {
            tokio::select! {
                output = operation => Ok(output),
                () = cancellation.cancelled() => Err(Boundary::Canceled),
                () = sender.closed() => {
                    cancellation.cancel();
                    Err(Boundary::Closed)
                }
                () = tokio::time::sleep_until(deadline.into()) => {
                    cancellation.cancel();
                    Err(Boundary::Deadline)
                }
            }
        }
        None => {
            tokio::select! {
                output = operation => Ok(output),
                () = cancellation.cancelled() => Err(Boundary::Canceled),
                () = sender.closed() => {
                    cancellation.cancel();
                    Err(Boundary::Closed)
                }
            }
        }
    }
}

async fn send_result<Cancellation>(
    cancellation: &Cancellation,
    sender: &mpsc::Sender<Result<ArtifactChunk, ArtifactError>>,
    deadline: Option<Instant>,
    result: Result<ArtifactChunk, ArtifactError>,
) -> bool
where
    Cancellation: ArtifactCancellation,
{
    match run_with_boundary(cancellation, sender, deadline, sender.send(result)).await {
        Ok(Ok(())) => true,
        Ok(Err(_)) | Err(Boundary::Closed | Boundary::Canceled) => false,
        Err(Boundary::Deadline) => {
            cancellation.cancel();
            send_deadline_error(sender);
            false
        }
    }
}

async fn send_error<Cancellation>(
    cancellation: &Cancellation,
    sender: &mpsc::Sender<Result<ArtifactChunk, ArtifactError>>,
    deadline: Option<Instant>,
    error: ArtifactError,
) where
    Cancellation: ArtifactCancellation,
{
    let _ = send_result(cancellation, sender, deadline, Err(error)).await;
}

fn send_deadline_error(sender: &mpsc::Sender<Result<ArtifactChunk, ArtifactError>>) {
    let _ = sender.try_send(Err(ArtifactError::DeadlineExceeded));
}

async fn stream_file<Cancellation>(
    mut file: File,
    sender: mpsc::Sender<Result<ArtifactChunk, ArtifactError>>,
    mut state: StreamState,
    cancellation: Cancellation,
    deadline: Option<Instant>,
) where
    Cancellation: ArtifactCancellation,
{
    let mut sent = 0_u64;
    let mut sequence = 0_u64;
    if state.offset == state.size_bytes {
        let cursor = encode_cursor(
            &state.cursor_key,
            state.actor_id,
            state.artifact_id,
            state.offset,
        );
        let _ = send_result(
            &cancellation,
            &sender,
            deadline,
            Ok(ArtifactChunk {
                sequence,
                contents: Vec::new(),
                committed_cursor: cursor,
                end_of_artifact: true,
                media_type: state.media_type,
            }),
        )
        .await;
        return;
    }
    while sent < state.total_limit && state.offset < state.size_bytes {
        let remaining = (state.total_limit - sent).min(state.size_bytes - state.offset);
        let length = remaining.min(u64::from(state.chunk_limit));
        let Ok(length) = usize::try_from(length) else {
            send_error(
                &cancellation,
                &sender,
                deadline,
                ArtifactError::ResourceExhausted,
            )
            .await;
            return;
        };
        let mut contents = vec![0_u8; length];
        match run_with_boundary(
            &cancellation,
            &sender,
            deadline,
            file.read_exact(&mut contents),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                send_error(&cancellation, &sender, deadline, ArtifactError::Io(error)).await;
                return;
            }
            Err(Boundary::Deadline) => {
                send_deadline_error(&sender);
                return;
            }
            Err(Boundary::Closed | Boundary::Canceled) => return,
        }
        let Ok(length) = u64::try_from(contents.len()) else {
            send_error(
                &cancellation,
                &sender,
                deadline,
                ArtifactError::ResourceExhausted,
            )
            .await;
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
        if !send_result(
            &cancellation,
            &sender,
            deadline,
            Ok(ArtifactChunk {
                sequence,
                contents,
                committed_cursor: cursor,
                end_of_artifact,
                media_type: state.media_type.clone(),
            }),
        )
        .await
        {
            return;
        }
        sequence += 1;
    }
}

#[cfg(test)]
#[path = "stream/tests.rs"]
mod tests;
