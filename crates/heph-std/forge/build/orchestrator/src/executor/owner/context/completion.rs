//! Exact publication context; cold imported data never adopts physical custody.
use super::super::node::{ClaimPins, State};
use super::{BuildExecutionError, ClaimedBuild, Node, Workspace, lock};
use crate::executor::{BuildExecutionResult, ReleaseArtifactInput};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Evidence {
    ImportedRepository,
    Completion,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    OriginalWorkspace,
    ColdImported,
}
pub(in super::super) struct Completion {
    original: ClaimPins,
    artifacts: Vec<ReleaseArtifactInput>,
    kind: Kind,
}
impl Completion {
    pub(in super::super) fn clears_finalized_hold(&self, state: &State) -> bool {
        !state.finalization
            && self.kind == Kind::ColdImported
            && matches!(
                state.cleanup_error.as_deref(),
                Some(BuildExecutionError::FinalizationRequired)
            )
            && cold_imported(state)
            && state
                .claim
                .as_ref()
                .is_some_and(|claim| same_pins(&self.original, claim))
    }

    pub(super) fn check_result(
        &self,
        state: &State,
        result: &BuildExecutionResult,
    ) -> Result<(), BuildExecutionError> {
        let context_matches = match self.kind {
            Kind::OriginalWorkspace => matches!(state.workspace, Workspace::Original(_)),
            Kind::ColdImported => cold_imported(state),
        };
        if !context_matches
            || result.build_request_id != self.original.input.id
            || result.release_id != self.original.release
            || result.release_agent_id != self.original.agent
            || result.release_version != self.original.version
            || result.artifact_count != self.artifacts.len()
        {
            return Err(BuildExecutionError::StoredState);
        }
        Ok(())
    }
}

impl Node {
    pub(in super::super::super) fn require_original_finalization(
        &self,
    ) -> Result<(), BuildExecutionError> {
        if lock(&self.state)
            .completion
            .as_ref()
            .is_some_and(|context| context.kind == Kind::ColdImported)
        {
            return Err(BuildExecutionError::UnsafeWorkspace);
        }
        self.original_workspace()?.recheck()
    }

    /// Only the decoded imported repository branch can establish this context.
    pub(in super::super::super) fn prepare_imported_completion(
        &self,
        claimed: &ClaimedBuild,
        artifacts: &[ReleaseArtifactInput],
    ) -> Result<(), BuildExecutionError> {
        self.bind_completion(claimed, artifacts, Evidence::ImportedRepository)
    }

    pub(in super::super::super) fn prepare_completion(
        &self,
        claimed: &ClaimedBuild,
        artifacts: &[ReleaseArtifactInput],
    ) -> Result<(), BuildExecutionError> {
        self.bind_completion(claimed, artifacts, Evidence::Completion)
    }

    fn bind_completion(
        &self,
        claimed: &ClaimedBuild,
        artifacts: &[ReleaseArtifactInput],
        evidence: Evidence,
    ) -> Result<(), BuildExecutionError> {
        self.claim(claimed)?;
        let mut state = lock(&self.state);
        if !state.detached || artifacts.is_empty() {
            return Err(BuildExecutionError::StoredState);
        }
        let (kind, pin) = match &state.workspace {
            Workspace::Original(pin) => (Kind::OriginalWorkspace, Some(pin.clone())),
            Workspace::Untouched
                if cold_imported(&state)
                    && (evidence == Evidence::ImportedRepository
                        || state
                            .completion
                            .as_ref()
                            .is_some_and(|context| context.kind == Kind::ColdImported)) =>
            {
                (Kind::ColdImported, None)
            }
            _ => return Err(BuildExecutionError::UnsafeWorkspace),
        };
        if let Some(original) = &state.completion {
            if original.kind != kind
                || !same_claim(&original.original, claimed)
                || !same_artifacts(&original.artifacts, artifacts)
            {
                return Err(BuildExecutionError::StoredState);
            }
        } else {
            state.completion = Some(Arc::new(Completion {
                original: ClaimPins {
                    input: claimed.input.clone(),
                    release: claimed.release_id,
                    agent: claimed.release_agent_id,
                    version: claimed.release_version.clone(),
                },
                artifacts: artifacts.to_vec(),
                kind,
            }));
        }
        drop(state);
        if let Some(pin) = pin {
            pin.recheck()?;
        }
        Ok(())
    }
}

fn cold_imported(state: &State) -> bool {
    let cleanup_known = match state.cleanup_error.as_deref() {
        None => true,
        Some(BuildExecutionError::FinalizationRequired) => state
            .completion
            .as_ref()
            .is_some_and(|context| context.kind == Kind::ColdImported),
        _ => false,
    };
    matches!(state.workspace, Workspace::Untouched)
        && state.detached
        && state.attempt.is_none()
        && state.instance.is_none()
        && state.preparation.is_none()
        && state.git_children.is_empty()
        && state.cleanup_failure.is_none()
        && cleanup_known
}

fn same_claim(original: &ClaimPins, candidate: &ClaimedBuild) -> bool {
    original.input.id == candidate.input.id
        && original.input.repository_id == candidate.input.repository_id
        && original.input.source_commit == candidate.input.source_commit
        && original.input.source_ref == candidate.input.source_ref
        && original.input.build == candidate.input.build
        && original.input.image_reference == candidate.input.image_reference
        && (original.release, original.agent, &original.version)
            == (
                candidate.release_id,
                candidate.release_agent_id,
                &candidate.release_version,
            )
}

fn same_artifacts(original: &[ReleaseArtifactInput], candidate: &[ReleaseArtifactInput]) -> bool {
    original.len() == candidate.len()
        && original.iter().zip(candidate).all(|(old, new)| {
            old.id == new.id
                && old.path == new.path
                && old.kind == new.kind
                && old.mode == new.mode
                && old.content_hash == new.content_hash
                && old.size_bytes == new.size_bytes
                && old.media_type == new.media_type
                && old.storage_key == new.storage_key
        })
}

fn same_pins(original: &ClaimPins, current: &ClaimPins) -> bool {
    original.input.id == current.input.id
        && original.input.repository_id == current.input.repository_id
        && original.input.source_commit == current.input.source_commit
        && original.input.source_ref == current.input.source_ref
        && original.input.build == current.input.build
        && original.input.image_reference == current.input.image_reference
        && original.release == current.release
        && original.agent == current.agent
        && original.version == current.version
}
