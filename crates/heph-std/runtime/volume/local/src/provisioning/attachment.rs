//! Read-only inspection of already-provisioned backing for an exact live lease.

use std::os::unix::fs::FileExt;

use volume_domain::VolumeAccessMode;
use volume_trait::{Volume, VolumeError, VolumeProvisioningState, VolumeState};

use super::{open_existing, open_root, prove_ext4, read_u32, validate_file, validate_intent};
use crate::{LocalVolumeStore, backing};

impl LocalVolumeStore {
    pub(crate) fn verify_run_backing(
        &self,
        volume: &Volume,
        mode: VolumeAccessMode,
    ) -> Result<(), VolumeError> {
        validate_intent(volume, &self.config.volume_root, &self.config.host_id)?;
        if volume.provisioning_state != VolumeProvisioningState::Ready
            || !matches!(volume.state, VolumeState::Ready | VolumeState::Attached)
        {
            return Err(VolumeError::InvalidState(
                "exact attachment requires proven ready backing",
            ));
        }
        let root = open_root(&self.config.volume_root)?;
        let file = open_existing(&root, &format!("{}.raw", volume.id)).map_err(backing)?;
        validate_file(&file)?;
        prove_ext4(&file, volume.capacity_bytes, volume.filesystem_uuid)?;
        let mut superblock = [0_u8; 1024];
        file.read_exact_at(&mut superblock, 1024).map_err(backing)?;
        let state = u16::from_le_bytes([superblock[58], superblock[59]]);
        let incompat = read_u32(&superblock, 96);
        if state & 2 != 0 || incompat & 8 != 0 {
            return Err(VolumeError::ProvisioningUncertain(
                "filesystem reports errors or external journal",
            ));
        }
        if mode == VolumeAccessMode::ReadOnly && (state & 1 == 0 || incompat & 4 != 0) {
            return Err(VolumeError::ProvisioningUncertain(
                "read-only attachment requires a clean ext4 filesystem",
            ));
        }
        // Ready backing was previously proven during provisioning. A known RW
        // filesystem may need normal journal replay after a fenced VM crash.
        // Do not run readonly fsck, format, resize, or clear journal flags here.
        Ok(())
    }
}
