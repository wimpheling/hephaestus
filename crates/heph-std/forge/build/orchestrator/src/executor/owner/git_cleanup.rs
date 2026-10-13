use super::{BuildExecutionError, lock, node::Node};
use std::sync::Arc;
struct Job {
    handle: tokio::task::JoinHandle<Result<(), BuildExecutionError>>,
    outcome: Option<Result<(), Arc<BuildExecutionError>>>,
}
pub(super) struct GitCleanup(tokio::sync::Mutex<Job>);
impl GitCleanup {
    pub(super) fn settled(&self) -> bool {
        self.0
            .try_lock()
            .is_ok_and(|job| job.outcome.is_some() && job.handle.is_finished())
    }
    async fn join(&self, owner: &Node) -> Result<(), BuildExecutionError> {
        let mut job = self.0.lock().await;
        if let Some(outcome) = &job.outcome {
            return outcome.clone().map_err(BuildExecutionError::Retained);
        }
        let result = match (&mut job.handle).await {
            Ok(result) => result.map_err(Arc::new),
            Err(error) => {
                owner.error(Arc::new(error));
                Err(Arc::new(BuildExecutionError::WorkerJoin))
            }
        };
        job.outcome = Some(result.clone());
        drop(job);
        result.map_err(BuildExecutionError::Retained)
    }
}
impl Node {
    pub(super) async fn stop_git(self: &Arc<Self>) -> Result<(), BuildExecutionError> {
        // Previously admitted cleanup tasks remain the originals. Join them
        // before a retry; retain errors, but retry actual Child/IO after failure.
        let previous = lock(&self.git_cleanup).clone();
        for job in previous {
            if let Err(error) = job.join(self).await {
                self.error(Arc::new(error));
            }
        }
        let records = lock(&self.state).git_children.clone();
        if records.is_empty() {
            return Ok(());
        }
        let job = {
            let mut retained = lock(&self.git_cleanup);
            let owner = self.clone();
            let handle = tokio::task::spawn_blocking(move || {
                for original in records {
                    if let Err(error) = original.stop_and_join() {
                        owner.error(Arc::new(error));
                        return Err(BuildExecutionError::WorkerJoin);
                    }
                }
                Ok(())
            });
            let job = Arc::new(GitCleanup(tokio::sync::Mutex::new(Job {
                handle,
                outcome: None,
            })));
            retained.push(job.clone());
            job
        };
        job.join(self).await
    }
}
