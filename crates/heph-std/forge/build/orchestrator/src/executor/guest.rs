//! Original attempt installation, physical detach and stable failure outcomes.
use super::{
    BuildExecutionError, BuildExecutor, ClaimedBuild, PreparedBuildWorkspace, StopMode,
    collect_execution, owner::Node,
};
use serde_json::Value;
use std::sync::Arc;
use vm_trait::{VmExit, VmInstance};

impl BuildExecutor {
    async fn build_failure(
        &self,
        node: &Node,
        verification: bool,
        code: &str,
    ) -> Result<(), BuildExecutionError> {
        if verification {
            self.repository
                .fail_verification(node.id(), code)
                .await
                .map_err(Into::into)
        } else {
            self.fail(node.id(), code, &[], &[]).await
        }
    }
    async fn provision_build(
        &self,
        claimed: &ClaimedBuild,
        workspace: &PreparedBuildWorkspace,
        node: &Arc<Node>,
        verification: bool,
    ) -> Result<Arc<dyn VmInstance>, BuildExecutionError> {
        let spec = match self.vm_spec(claimed, workspace) {
            Ok(spec) => spec,
            Err(error) => {
                self.build_failure(node, verification, "invalid_build_contract")
                    .await?;
                return Err(error);
            }
        };
        let attempt = match self.provider.prepare_provision_attempt(spec) {
            Ok(attempt) => attempt,
            Err(error) => {
                node.vm_error(error);
                self.build_failure(node, verification, "vm_provision")
                    .await?;
                return Err(BuildExecutionError::Vm);
            }
        };
        // Original opaque attempt installed BEFORE polling effect-capable IO.
        node.install_attempt(attempt.clone())?;
        let instance = match attempt.provision().await {
            Ok(instance) => instance,
            Err(error) => {
                node.vm_error(error);
                node.detach().await?;
                self.build_failure(node, verification, "vm_provision")
                    .await?;
                return Err(BuildExecutionError::Vm);
            }
        };
        node.instance(instance.clone());
        Ok(instance)
    }
    pub(super) async fn build_guest(
        &self,
        claimed: &ClaimedBuild,
        workspace: &PreparedBuildWorkspace,
        node: &Arc<Node>,
        verification: bool,
    ) -> Result<(VmExit, Vec<Value>, Vec<Value>), BuildExecutionError> {
        let instance = self
            .provision_build(claimed, workspace, node, verification)
            .await?;
        let mut events = instance.subscribe_events();
        node.check_open()?;
        if let Err(error) = instance.start().await {
            node.vm_error(error);
            node.detach().await?;
            self.build_failure(node, verification, "vm_start").await?;
            return Err(BuildExecutionError::Vm);
        }
        if !verification && let Err(error) = self.mark_running(node.id()).await {
            let original = Arc::new(error);
            node.error(original.clone());
            node.detach().await?;
            return Err(BuildExecutionError::Retained(original));
        }
        let (exit, logs, metrics, timed_out) =
            collect_execution(&instance, &mut events, self.config.timeout, node).await;
        if timed_out && let Err(error) = instance.stop(StopMode::Force).await {
            node.vm_error(error);
        }
        // No seal/import/workspace removal can precede original token cleanup.
        if let Err(error) = node.detach().await {
            self.build_failure(node, verification, "vm_destroy").await?;
            return Err(error);
        }
        let exit = exit.ok_or(BuildExecutionError::Vm)?;
        if timed_out || exit.code != Some(0) || exit.signal.is_some() {
            if verification {
                self.repository
                    .fail_verification(node.id(), "guest_failed")
                    .await?;
            } else {
                self.fail_with_exit(node.id(), "guest_failed", &exit, &logs, &metrics)
                    .await?;
            }
            node.remove_workspace()?;
            return Err(BuildExecutionError::GuestFailed);
        }
        Ok((exit, logs, metrics))
    }
}
