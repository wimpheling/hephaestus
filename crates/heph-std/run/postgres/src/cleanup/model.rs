use run_domain::{RunCleanupHostId, RunCleanupLeaseFence, RunCleanupTarget, RunCleanupVmTarget};
use run_orchestrator::RepositoryError;
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, LeaseId, RunId, VolumeId};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::VmId;

use crate::errors::storage;

#[derive(FromRow)]
pub struct Execution {
    pub instance_id: Uuid,
    pub instance_revision_id: Uuid,
    pub state: String,
    pub vm_id: Option<String>,
    pub vm_provider_namespace: Option<String>,
    pub vm_host_id: Option<String>,
    pub vm_binding_kind: Option<String>,
    pub volume_acquisition_closed_at: Option<OffsetDateTime>,
}

impl Execution {
    pub fn vm_target(&self) -> Result<RunCleanupVmTarget, RepositoryError> {
        if self.vm_binding_kind.is_none()
            && self.vm_provider_namespace.is_none()
            && self.vm_host_id.is_none()
        {
            // A historical VM label does not establish its provider ownership.
            return Ok(RunCleanupVmTarget::historical_unresolved());
        }
        vm_target(
            self.vm_binding_kind
                .as_deref()
                .unwrap_or("historical_unresolved"),
            self.vm_provider_namespace.clone(),
            self.vm_host_id.clone(),
            self.vm_id.clone(),
        )
    }
}

#[derive(FromRow)]
pub struct TargetRow {
    pub run_id: Uuid,
    pub instance_id: Uuid,
    pub instance_revision_id: Uuid,
    pub generation: i64,
    pub vm_target_kind: String,
    pub vm_provider_namespace: Option<String>,
    pub vm_host_id: Option<String>,
    pub vm_id: Option<String>,
    pub target_digest: Vec<u8>,
}

impl TargetRow {
    pub fn target(
        self,
        leases: Vec<RunCleanupLeaseFence>,
    ) -> Result<RunCleanupTarget, RepositoryError> {
        let target = RunCleanupTarget::new(
            RunId::from_uuid(self.run_id),
            AgentInstanceId::from_uuid(self.instance_id),
            AgentInstanceRevisionId::from_uuid(self.instance_revision_id),
            vm_target(
                &self.vm_target_kind,
                self.vm_provider_namespace,
                self.vm_host_id,
                self.vm_id,
            )?,
            u64::try_from(self.generation).map_err(storage)?,
            leases,
        )
        .map_err(storage)?;
        if self.target_digest.as_slice() != target.digest().as_slice() {
            return Err(RepositoryError::InvalidData(
                "cleanup target digest is inconsistent",
            ));
        }
        Ok(target)
    }
}

#[derive(FromRow)]
pub struct FenceRow {
    pub run_id: Uuid,
    pub lease_id: Uuid,
    pub volume_id: Uuid,
    pub host_id: String,
    pub fencing_token: i64,
}

impl TryFrom<FenceRow> for RunCleanupLeaseFence {
    type Error = RepositoryError;
    fn try_from(row: FenceRow) -> Result<Self, Self::Error> {
        Self::new(
            RunId::from_uuid(row.run_id),
            LeaseId::from_uuid(row.lease_id),
            VolumeId::from_uuid(row.volume_id),
            row.host_id,
            row.fencing_token,
        )
        .map_err(storage)
    }
}

#[derive(FromRow)]
pub struct ReceiptRow {
    pub observation_kind: String,
    pub vm_provider_namespace: String,
    pub vm_host_id: String,
    pub vm_id: String,
    pub observed_at: OffsetDateTime,
    pub target_digest: Vec<u8>,
}

fn vm_target(
    kind: &str,
    namespace: Option<String>,
    host: Option<String>,
    vm: Option<String>,
) -> Result<RunCleanupVmTarget, RepositoryError> {
    if kind == "historical_unresolved" {
        if namespace.is_some() || host.is_some() || vm.is_some() {
            return Err(RepositoryError::InvalidData(
                "unresolved cleanup target cannot claim a provider identity",
            ));
        }
        return Ok(RunCleanupVmTarget::historical_unresolved());
    }
    let scope = RunCleanupHostId::new(
        namespace.ok_or(RepositoryError::InvalidData("cleanup namespace is missing"))?,
        host.ok_or(RepositoryError::InvalidData("cleanup host is missing"))?,
    )
    .map_err(storage)?;
    let vm = VmId(vm.ok_or(RepositoryError::InvalidData("cleanup VM is missing"))?);
    match kind {
        "planned" => RunCleanupVmTarget::planned(scope, vm).map_err(storage),
        "persisted" => RunCleanupVmTarget::persisted(scope, vm).map_err(storage),
        _ => Err(RepositoryError::InvalidData(
            "cleanup VM provenance is invalid",
        )),
    }
}
