//! Full original build/finalization comparison; saved input is never authority.
use super::super::workspace::pin::WorkspacePin;
use super::{
    BuildExecutionError, lock,
    node::{ClaimPins, Node, Workspace},
};
use crate::ClaimedBuild;
use std::sync::Arc;
impl Node {
    pub(in super::super) fn claim(
        &self,
        claimed: &ClaimedBuild,
    ) -> Result<(), BuildExecutionError> {
        let mut state = lock(&self.state);
        if claimed.input.id != self.id {
            return Err(BuildExecutionError::StoredState);
        }
        if let Some(old) = &state.claim {
            let input = &claimed.input;
            if old.input.id != input.id
                || old.input.repository_id != input.repository_id
                || old.input.source_commit != input.source_commit
                || old.input.source_ref != input.source_ref
                || old.input.build != input.build
                || old.input.image_reference != input.image_reference
                || old.release != claimed.release_id
                || old.agent != claimed.release_agent_id
                || old.version != claimed.release_version
            {
                return Err(BuildExecutionError::StoredState);
            }
        } else {
            state.claim = Some(ClaimPins {
                input: claimed.input.clone(),
                release: claimed.release_id,
                agent: claimed.release_agent_id,
                version: claimed.release_version.clone(),
            });
        }
        drop(state);
        Ok(())
    }
    pub(in super::super) fn acquire_finalization_workspace(
        &self,
        path: &std::path::Path,
        root: &Arc<WorkspacePin>,
    ) -> Result<(), BuildExecutionError> {
        if matches!(lock(&self.state).workspace, Workspace::Untouched) {
            self.begin_workspace(path.to_path_buf())?;
            self.workspace(WorkspacePin::open(path, Some(root.clone()))?);
        }
        self.original_workspace()?.recheck()
    }
    pub(in super::super) fn completed_result(
        &self,
        result: &super::BuildExecutionResult,
    ) -> Result<(), BuildExecutionError> {
        let state = lock(&self.state);
        let Some(claim) = &state.claim else {
            return if result.build_request_id == self.id
                && !state.finalization
                && state.attempt.is_none()
                && matches!(state.workspace, Workspace::Untouched)
            {
                Ok(())
            } else {
                Err(BuildExecutionError::StoredState)
            };
        };
        if result.build_request_id != self.id
            || result.release_id != claim.release
            || result.release_agent_id != claim.agent
            || result.release_version != claim.version
        {
            return Err(BuildExecutionError::StoredState);
        }
        drop(state);
        Ok(())
    }
}
