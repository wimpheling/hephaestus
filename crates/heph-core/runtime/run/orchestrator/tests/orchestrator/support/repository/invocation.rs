//! Exact controlled 117 consumption; no SQL receipt or native qualification.
use super::{
    InvocationCancelEvidence, MemoryRepository, Ordering, RepositoryError, RunId,
    VmProviderOwnerScope,
};
use run_domain::{RunCleanupHostId, RunKind};
use vm_trait::VmId;

impl MemoryRepository {
    pub(super) async fn consumed_invocation(
        &self,
        id: RunId,
        scope: &VmProviderOwnerScope,
    ) -> Result<InvocationCancelEvidence, RepositoryError> {
        let created = *self.created.lock().await;
        let plan = self.planned_vm.lock().await.clone();
        let run = self.run.lock().await.clone();
        let consumed = self.invocation_consumed.lock().await.clone();
        let host = RunCleanupHostId::new(scope.namespace().into(), scope.host_id().into())
            .map_err(|_| RepositoryError::InvalidData("controlled consumed owner differs"))?;
        if !self.invocation_admitted.load(Ordering::SeqCst)
            || !created
            || run.id != id
            || run.kind != RunKind::Invocation
            || run.attachment_id.is_some()
            || run.volume_id.is_some()
            || run.lease_id.is_some()
            || run.lease_fencing_token.is_some()
            || run.vm_id.as_deref() != Some(id.to_string().as_str())
            || plan.as_ref() != Some(&(host, VmId(id.to_string())))
        {
            return Err(RepositoryError::InvalidData(
                "controlled checked Invocation consumption is absent or differs",
            ));
        }
        let original = consumed.ok_or(RepositoryError::InvalidData(
            "controlled checked Invocation consumption is absent",
        ))?;
        if (run.id, run.kind) != (original.run_id, original.kind)
            || run.instance_id != original.instance_id
            || run.instance_revision_id != original.instance_revision_id
            || run.release_id != original.release_id
            || run.release_agent_id != original.release_agent_id
            || run.command_id != original.command_id
            || run.requires_state != original.requires_state
            || run.attachment_id != original.attachment_id
        {
            return Err(RepositoryError::InvalidData(
                "controlled consumed original differs",
            ));
        }
        Ok(InvocationCancelEvidence::Consumed117)
    }
}
