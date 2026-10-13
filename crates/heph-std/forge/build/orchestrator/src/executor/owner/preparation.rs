use super::super::{PreparedBuildWorkspace, workspace::prepare_registered};
use super::{BuildExecutionError, BuildExecutor, lock, node::Node};
use crate::ClaimedBuild;
use std::sync::Arc;

struct Job {
    handle: tokio::task::JoinHandle<Result<PreparedBuildWorkspace, BuildExecutionError>>,
    result: Option<Result<Arc<PreparedBuildWorkspace>, Arc<BuildExecutionError>>>,
}
pub(super) struct Preparation {
    job: tokio::sync::Mutex<Job>,
    node: std::sync::Weak<Node>,
}
impl Preparation {
    pub(super) async fn join(
        &self,
    ) -> Result<Arc<PreparedBuildWorkspace>, Arc<BuildExecutionError>> {
        let mut job = self.job.lock().await;
        if let Some(result) = &job.result {
            return result.clone();
        }
        let result = match (&mut job.handle).await {
            Ok(result) => result.map(Arc::new).map_err(Arc::new),
            Err(error) => {
                if let Some(node) = self.node.upgrade() {
                    node.error(Arc::new(error));
                }
                Err(Arc::new(BuildExecutionError::WorkerJoin))
            }
        };
        job.result = Some(result.clone());
        result
    }
}
impl BuildExecutor {
    pub(in super::super) async fn materialize(
        &self,
        claimed: &ClaimedBuild,
        node: &Arc<Node>,
    ) -> Result<Arc<PreparedBuildWorkspace>, BuildExecutionError> {
        node.claim(claimed)?;
        node.check_open()?;
        let config = self.config.clone();
        let input = claimed.input.clone();
        let workspace = self.active_path(node.id);
        let owner = node.clone();
        let root = self.workspace_root.clone();
        let handle = tokio::task::spawn_blocking(move || {
            prepare_registered(&config, &input, &workspace, &owner, &root)
        });
        let preparation = Arc::new(Preparation {
            job: tokio::sync::Mutex::new(Job {
                handle,
                result: None,
            }),
            node: Arc::downgrade(node),
        });
        // No await between actual blocking spawn and registry-held handle.
        lock(&node.state).preparation = Some(preparation.clone());
        preparation
            .join()
            .await
            .map_err(BuildExecutionError::Retained)
    }
}
