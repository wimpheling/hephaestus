use run_domain::Run;
use vm_trait::{DiskFormat, VmDisk, VmError, VmGuestVolume, VmSpec, VmVolumeInitializationPurpose};
use volume_domain::{VolumeAccessMode, VolumeSelectionOrigin};
use volume_trait::INSTANCE_STATE_DISK_ID;

use super::{RunOrchestrator, plural_evidence::PreparedRunVolumes};

impl RunOrchestrator {
    pub(super) async fn build_plural_spec(
        &self,
        run: &Run,
        volumes: &PreparedRunVolumes,
        mounts: Vec<vm_trait::VmMount>,
    ) -> Result<VmSpec, VmError> {
        let mut spec = self
            .spec_factory
            .build_with_volumes(run, &volumes.selections)
            .await?;
        if run.vm_id.as_deref() != Some(spec.id.0.as_str()) {
            return Err(VmError::InvalidState(
                "plural VM spec differs from durable planned identity",
            ));
        }
        // Factories return the base workload; persisted selections own all volume metadata.
        spec.disks.retain(|disk| disk.id != INSTANCE_STATE_DISK_ID);
        spec.guest_volumes.clear();
        spec.labels.remove("hephaestus.agent-state.filesystem-uuid");
        spec.labels.remove("hephaestus.agent-state.mount-path");
        for attachment in &volumes.attachments {
            let selection = attachment
                .lease
                .selection()
                .ok_or(VmError::InvalidState("unproven guest attachment"))?;
            if spec.disks.iter().any(|disk| disk.id == attachment.disk_id) {
                return Err(VmError::InvalidState(
                    "selected disk conflicts with workload disk",
                ));
            }
            let disk_id = if selection.origin() == VolumeSelectionOrigin::LegacyOrigin {
                INSTANCE_STATE_DISK_ID.into()
            } else {
                attachment.disk_id.clone()
            };
            spec.disks.push(VmDisk {
                id: disk_id.clone(),
                host_path: attachment.volume.host_path.clone(),
                format: DiskFormat::Raw,
                read_only: selection.scope().access_mode() == VolumeAccessMode::ReadOnly,
            });
            let builtin_state = matches!(
                selection.origin(),
                VolumeSelectionOrigin::LegacyOrigin | VolumeSelectionOrigin::LegacyDeclaration
            );
            let purpose = if builtin_state {
                VmVolumeInitializationPurpose::BuiltinStateSQLite
            } else {
                VmVolumeInitializationPurpose::None
            };
            spec.guest_volumes.push(
                VmGuestVolume::new(
                    selection.scope().slot().clone(),
                    disk_id,
                    attachment.volume.filesystem_uuid,
                    selection.declaration().guest_path().clone(),
                    selection.scope().access_mode(),
                )?
                .with_initialization_purpose(purpose, builtin_state)?,
            );
        }
        spec.mounts.extend(mounts);
        vm_trait::validate_vm_guest_volumes(&spec)?;
        Ok(spec)
    }
}
