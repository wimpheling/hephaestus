use super::{
    BuildExecutionError, BuildExecutionResult, BuildExecutor, BuildRequestId, lock,
    models::{Mode, Outcome},
    node::Node,
    registry::Admission,
};
use std::sync::Arc;

pub(super) async fn execute(
    executor: BuildExecutor,
    id: BuildRequestId,
    mode: Mode,
) -> Result<BuildExecutionResult, BuildExecutionError> {
    if let Some(original) = executor.registry.get(id) {
        // Existing custody already registered: observe only completed original
        // joins before same-ID admission/finalization, never poll running work.
        original.join_finished_jobs().await;
    }
    let node = admit(executor, id, mode)?;
    match node.outcome().await {
        Outcome::Build(result) => result.map_err(observed_error),
        Outcome::Verify(_) => Err(BuildExecutionError::StoredState),
    }
}
pub(super) async fn verify(
    executor: BuildExecutor,
    id: BuildRequestId,
) -> Result<(), BuildExecutionError> {
    if let Some(original) = executor.registry.get(id) {
        original.join_finished_jobs().await;
    }
    let node = admit(executor, id, Mode::Verify)?;
    match node.outcome().await {
        Outcome::Verify(result) => result.map_err(observed_error),
        Outcome::Build(_) => Err(BuildExecutionError::StoredState),
    }
}
fn admit(
    executor: BuildExecutor,
    id: BuildRequestId,
    mode: Mode,
) -> Result<Arc<Node>, BuildExecutionError> {
    match executor.registry.register(id, mode)? {
        Admission::Follow(node) => Ok(node),
        Admission::Start(node, mode) => {
            let owner = node.clone();
            start(&node, async move {
                match mode {
                    Mode::Execute => {
                        Outcome::Build(executor.execute_owned(id, &owner).await.map_err(Arc::new))
                    }
                    Mode::Retry => {
                        let result = async {
                            owner.check_open()?;
                            executor.ensure_image_available(id).await?;
                            owner.check_open()?;
                            executor.repository.reset_for_retry(id).await?;
                            executor.execute_owned(id, &owner).await
                        }
                        .await;
                        Outcome::Build(result.map_err(Arc::new))
                    }
                    Mode::Verify => {
                        Outcome::Verify(executor.verify_owned(id, &owner).await.map_err(Arc::new))
                    }
                    Mode::Finalize => {
                        let result = executor
                            .resume_finalization(id, &owner)
                            .await
                            .and_then(|result| result.ok_or(BuildExecutionError::StoredState));
                        Outcome::Build(result.map_err(Arc::new))
                    }
                }
            });
            Ok(node)
        }
    }
}
// Registration precedes this spawn and first await. The retained watcher joins
// the actual execution task, including panic, independently of every waiter.
pub(super) fn start(
    node: &Arc<Node>,
    work: impl std::future::Future<Output = Outcome> + Send + 'static,
) {
    let (release, admission) = tokio::sync::oneshot::channel();
    let worker_owner = node.clone();
    let actual = super::task::Task::new(tokio::spawn(async move {
        if let Err(error) = admission.await {
            worker_owner.error(Arc::new(error));
            let error = Arc::new(BuildExecutionError::WorkerJoin);
            return if lock(&worker_owner.state).mode == Mode::Verify {
                Outcome::Verify(Err(error))
            } else {
                Outcome::Build(Err(error))
            };
        }
        work.await
    }));
    *lock(&node.work_job) = Some(actual.clone());
    let owner = node.clone();
    let watcher = super::task::Task::new(tokio::spawn(async move {
        let outcome = actual.join(&owner).await.map_or_else(
            |_| {
                let error = Arc::new(BuildExecutionError::WorkerJoin);
                if lock(&owner.state).mode == Mode::Verify {
                    Outcome::Verify(Err(error))
                } else {
                    Outcome::Build(Err(error))
                }
            },
            std::convert::identity,
        );
        finish(&owner, &actual, outcome);
        #[cfg(test)]
        {
            let control = { lock(&owner.watcher_control).clone() };
            if let Some(control) = control {
                control.entered.notify_one();
                control.released.notified().await;
                assert!(
                    !control.panic,
                    "controlled outer watcher panic after work join"
                );
            }
        }
    }));
    *lock(&node.execution_job) = Some(watcher);
    // Actual work cannot poll engine IO until BOTH original handles are saved.
    let _released = release.send(());
}
pub(super) fn finish(
    owner: &Arc<Node>,
    actual: &Arc<super::task::Task<Outcome>>,
    outcome: Outcome,
) {
    let failed = outcome.failed();
    {
        let work = lock(&owner.work_job);
        if !work
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, actual))
        {
            return;
        }
        let mut state = lock(&owner.state);
        if state.outcome.is_some() {
            return;
        }
        match &outcome {
            Outcome::Build(Err(error)) | Outcome::Verify(Err(error)) => {
                state.errors.push(error.clone());
            }
            Outcome::Build(Ok(_)) | Outcome::Verify(Ok(())) => {}
        }
        state.outcome = Some(outcome);
        drop(state);
        drop(work);
    }
    if failed {
        owner.kick_cleanup(false);
    }
    owner.changed.notify_waiters();
}
// Preserve existing unit failure classes. Source-bearing failures retain the
// exact original boxed/Arc source instead of manufacturing a clone.
fn observed_error(error: Arc<BuildExecutionError>) -> BuildExecutionError {
    use BuildExecutionError::{
        AlreadyClaimed, Artifact, Authorization, Closed, Database, Filesystem, FilesystemSource,
        FinalizationRequired, Git, GitPrerequisite, GuestFailed, ImageUnavailable,
        InvalidConfiguration, InvalidGitTree, MissingOutput, NetworkDenied, Release, ReleaseValue,
        Retained, SourceQuota, StoredState, Unauthorized, Unavailable, UndeclaredOutput,
        UnsafeOutput, UnsafeRepository, UnsafeWorkspace, UnsupportedSourceObject,
        VerificationMismatch, Vm, VmCleanup, WorkerJoin,
    };
    match error.as_ref() {
        InvalidConfiguration => InvalidConfiguration,
        Unavailable => Unavailable,
        AlreadyClaimed => AlreadyClaimed,
        Unauthorized => Unauthorized,
        Authorization => Authorization,
        StoredState => StoredState,
        UnsafeRepository => UnsafeRepository,
        UnsafeWorkspace => UnsafeWorkspace,
        SourceQuota => SourceQuota,
        InvalidGitTree => InvalidGitTree,
        UnsupportedSourceObject => UnsupportedSourceObject,
        Git => Git,
        GitPrerequisite => GitPrerequisite,
        ImageUnavailable => ImageUnavailable,
        NetworkDenied => NetworkDenied,
        Vm => Vm,
        VmCleanup => VmCleanup,
        GuestFailed => GuestFailed,
        VerificationMismatch => VerificationMismatch,
        UnsafeOutput => UnsafeOutput,
        UndeclaredOutput => UndeclaredOutput,
        MissingOutput => MissingOutput,
        Release => Release,
        WorkerJoin => WorkerJoin,
        Filesystem | FilesystemSource(_) => Filesystem,
        Closed => Closed,
        FinalizationRequired => FinalizationRequired,
        Retained(original) => observed_error(original.clone()),
        Artifact(_) | Database(_) | ReleaseValue(_) => Retained(error),
    }
}
