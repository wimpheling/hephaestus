//! Explicit Invocation composition and defensive planned-admission checks.

use run_domain::{RunKind, StartRun};
use vm_trait::VmProviderOwnerScope;

use super::{OrchestratorError, RunOrchestrator};
use crate::{CreateRunResult, RepositoryError};

impl RunOrchestrator {
    pub(super) fn require_invocation_configuration(
        &self,
        command: &StartRun,
    ) -> Result<(), OrchestratorError> {
        let Some(expected) = self.invocation_scope.as_ref() else {
            return Err(OrchestratorError::InvocationUnsupported);
        };
        if !self.plural_volumes
            || self.canonical_cleanup.is_none()
            || self.legacy_scope.is_some()
            || command.attachment_id.is_some()
        {
            return Err(OrchestratorError::InvocationUnsupported);
        }
        if &self
            .provider
            .owner_scope()
            .map_err(|_| OrchestratorError::InvocationUnsupported)?
            != expected
        {
            return Err(OrchestratorError::InvocationUnsupported);
        }
        Ok(())
    }

    pub(super) fn validate_invocation_admission(
        &self,
        command: &StartRun,
        scope: &VmProviderOwnerScope,
        created: &CreateRunResult,
    ) -> Result<(), OrchestratorError> {
        if command.kind != RunKind::Invocation {
            return Ok(());
        }
        let run = &created.run;
        // Checked data alone cannot prove protected SQL lineage. The trusted
        // repository must consume it atomically; these checks reject a wrong
        // return before binding, acquisition or provider IO.
        let same_run = run.id == command.run_id;
        if self.invocation_scope.as_ref() != Some(scope)
            || created.created
            || !same_run
            || run.command_id != command.command_id
            || run.kind != command.kind
            || run.attachment_id.is_some()
            || run.instance_id != command.instance_id
            || run.instance_revision_id != command.instance_revision_id
            || run.release_id != command.release_id
            || run.release_agent_id != command.release_agent_id
            || run.requires_state != command.requires_state
            || run.vm_id.as_deref() != Some(command.run_id.to_string().as_str())
        {
            return Err(RepositoryError::InvalidData(
                "Invocation admission or provider plan differs",
            )
            .into());
        }
        Ok(())
    }
}
