//! Actual task custody before any cancellation point; waiter owns no Core future.
use super::{
    held,
    intake::{Operations, Request},
};
use run_orchestrator::{OrchestratorError, RepositoryError};
use std::{error::Error, fmt, future::Future, sync::Arc};
use tokio::sync::oneshot;

pub(super) fn spawn<T, F>(
    operations: &Arc<Operations>,
    request: Request,
    future: F,
) -> Result<oneshot::Receiver<Result<T, OrchestratorError>>, OrchestratorError>
where
    T: Send + 'static,
    F: Future<Output = Result<T, OrchestratorError>> + Send + 'static,
{
    let sequence = operations.register(request.clone())?;
    let owner = Arc::clone(operations);
    let (send, receive) = oneshot::channel();
    let (admit, admitted) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let mut completion = Completion {
            owner: Arc::clone(&owner),
            sequence,
            finished: false,
        };
        // Future captures the actual original controllers/backend/guards, not a waiter.
        let outcome = match admitted.await {
            Ok(()) => future.await,
            Err(_) => Err(held(
                "managed task admission handoff failed before execution",
            )),
        };
        let (outcome, failure) = match outcome {
            Ok(value) => (Ok(value), None),
            Err(cause) => {
                tracing::error!(run_id = ?request.run_id(), %cause, "managed original operation remains held");
                let original = Arc::new(cause);
                (Err(shared_error(Arc::clone(&original))), Some(original))
            }
        };
        match owner.finish(sequence, failure) {
            Ok(failure) => {
                completion.finished = true;
                drop(completion);
                let outcome = failure.map_or(outcome, |cause| Err(shared_error(cause)));
                drop(send.send(outcome));
            }
            Err(cause) => {
                drop(completion);
                drop(send.send(Err(cause)));
            }
        }
    });
    operations.attach(sequence, handle)?;
    // Actual handle is registered before the task may poll the Core pipeline.
    admit
        .send(())
        .map_err(|()| held("managed original task failed before execution handoff"))?;
    Ok(receive)
}

pub(super) async fn receive<T>(
    receiver: oneshot::Receiver<Result<T, OrchestratorError>>,
) -> Result<T, OrchestratorError> {
    receiver
        .await
        .map_err(|_| held("managed owning operation result channel closed; custody unresolved"))?
}

struct Completion {
    owner: Arc<Operations>,
    sequence: u64,
    finished: bool,
}
impl Drop for Completion {
    fn drop(&mut self) {
        if !self.finished {
            self.owner.unexpected(self.sequence);
        }
    }
}

#[derive(Debug)]
struct OriginalError(Arc<OrchestratorError>);
impl fmt::Display for OriginalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0.as_ref(), f)
    }
}
impl Error for OriginalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}
pub(super) fn shared_error(original: Arc<OrchestratorError>) -> OrchestratorError {
    // Retain this control-flow distinction while retaining every actual source.
    match original.as_ref() {
        OrchestratorError::RunInProgress(run) => return OrchestratorError::RunInProgress(*run),
        OrchestratorError::InvocationUnsupported => {
            return OrchestratorError::InvocationUnsupported;
        }
        _ => {}
    }
    RepositoryError::Storage(Box::new(OriginalError(original))).into()
}
