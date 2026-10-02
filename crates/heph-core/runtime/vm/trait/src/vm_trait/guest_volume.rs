use capability_domain::CapabilitySlotKey;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use volume_domain::{GuestMountPath, MAX_VOLUME_SLOTS, VolumeAccessMode};

use crate::{VmError, VmSpec};

#[path = "guest_initialization.rs"]
mod initialization;
pub use initialization::VmVolumeInitializationPurpose;

/// An exact guest attachment declaration referencing an existing VM disk.
///
/// This metadata carries no grant and contains no host path. The orchestrator
/// must resolve it from an authorized immutable revision before provisioning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "GuestVolumeWire")]
pub struct VmGuestVolume {
    slot: CapabilitySlotKey,
    disk_id: String,
    filesystem_uuid: Uuid,
    guest_path: GuestMountPath,
    access_mode: VolumeAccessMode,
    #[serde(
        default,
        skip_serializing_if = "VmVolumeInitializationPurpose::is_none"
    )]
    initialization_purpose: VmVolumeInitializationPurpose,
    #[serde(default, skip_serializing_if = "initialization::is_false")]
    frozen_requires_state: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestVolumeWire {
    slot: CapabilitySlotKey,
    disk_id: String,
    filesystem_uuid: Uuid,
    guest_path: GuestMountPath,
    access_mode: VolumeAccessMode,
    #[serde(default)]
    initialization_purpose: VmVolumeInitializationPurpose,
    #[serde(default)]
    frozen_requires_state: bool,
}

impl VmGuestVolume {
    /// Creates bounded attachment metadata without authorizing the resource.
    ///
    /// # Errors
    /// Rejects nil filesystem UUIDs and unbounded or ambiguous disk IDs.
    pub fn new(
        slot: CapabilitySlotKey,
        disk_id: impl Into<String>,
        filesystem_uuid: Uuid,
        guest_path: GuestMountPath,
        access_mode: VolumeAccessMode,
    ) -> Result<Self, VmError> {
        let disk_id = disk_id.into();
        if filesystem_uuid.is_nil()
            || disk_id.is_empty()
            || disk_id.len() > 64
            || !disk_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(invalid("invalid filesystem UUID or disk ID"));
        }
        Ok(Self {
            slot,
            disk_id,
            filesystem_uuid,
            guest_path,
            access_mode,
            initialization_purpose: VmVolumeInitializationPurpose::None,
            frozen_requires_state: false,
        })
    }

    /// Returns the immutable release slot key.
    #[must_use]
    pub const fn slot(&self) -> &CapabilitySlotKey {
        &self.slot
    }
    /// Returns the exact VM disk identifier.
    #[must_use]
    pub fn disk_id(&self) -> &str {
        &self.disk_id
    }
    /// Returns the filesystem identity used inside the guest.
    #[must_use]
    pub const fn filesystem_uuid(&self) -> Uuid {
        self.filesystem_uuid
    }
    /// Returns the controlled guest mount path.
    #[must_use]
    pub const fn guest_path(&self) -> &GuestMountPath {
        &self.guest_path
    }
    /// Returns the exact attachment access mode.
    #[must_use]
    pub const fn access_mode(&self) -> VolumeAccessMode {
        self.access_mode
    }
}

impl TryFrom<GuestVolumeWire> for VmGuestVolume {
    type Error = VmError;
    fn try_from(wire: GuestVolumeWire) -> Result<Self, Self::Error> {
        Self::new(
            wire.slot,
            wire.disk_id,
            wire.filesystem_uuid,
            wire.guest_path,
            wire.access_mode,
        )?
        .with_initialization_purpose(wire.initialization_purpose, wire.frozen_requires_state)
    }
}

/// Checks a bounded set for ambiguous slot, disk, filesystem, or mount aliases.
///
/// # Errors
/// Rejects aliases, overlapping paths, and excessive attachment counts.
pub fn validate_guest_volume_set(volumes: &[VmGuestVolume]) -> Result<(), VmError> {
    if volumes.len() > MAX_VOLUME_SLOTS {
        return Err(invalid("too many named guest volumes"));
    }
    for (index, volume) in volumes.iter().enumerate() {
        initialization::validate(volume)?;
        for previous in &volumes[..index] {
            if volume.slot == previous.slot
                || volume.disk_id == previous.disk_id
                || volume.filesystem_uuid == previous.filesystem_uuid
                || paths_overlap(volume.guest_path.as_str(), previous.guest_path.as_str())
            {
                return Err(invalid(
                    "duplicate attachment identity or overlapping mount path",
                ));
            }
        }
    }
    Ok(())
}

/// Decodes a bounded attachment set and rejects aliases before guest bootstrap.
///
/// # Errors
/// Rejects invalid entries, duplicate identities, path overlap, and excess count.
pub fn deserialize_guest_volumes<'de, D>(deserializer: D) -> Result<Vec<VmGuestVolume>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let volumes = Vec::<VmGuestVolume>::deserialize(deserializer)?;
    validate_guest_volume_set(&volumes).map_err(serde::de::Error::custom)?;
    Ok(volumes)
}

/// Checks exact disk modes and conflicts with other mounts and legacy labels.
///
/// # Errors
/// Rejects missing/duplicate disks, access mismatches, mount conflicts, and
/// mixed named/legacy attachment declarations.
pub fn validate_vm_guest_volumes(spec: &VmSpec) -> Result<(), VmError> {
    validate_guest_volume_set(&spec.guest_volumes)?;
    if spec.guest_volumes.is_empty() {
        return Ok(());
    }
    if spec.labels.keys().any(|key| {
        matches!(
            key.as_str(),
            "hephaestus.agent-state.filesystem-uuid"
                | "hephaestus.agent-state.mount-path"
                | "hephaestus.oci-scratch.filesystem-uuid"
                | "hephaestus.oci-scratch.mount-path"
        )
    }) {
        return Err(invalid(
            "named volumes cannot combine with legacy volume labels",
        ));
    }
    let paths = spec
        .mounts
        .iter()
        .map(|mount| {
            mount
                .guest_path
                .to_str()
                .ok_or_else(|| invalid("mount path is not UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_guest_volume_mounts(&spec.guest_volumes, &paths)?;
    for volume in &spec.guest_volumes {
        let mut disks = spec.disks.iter().filter(|disk| disk.id == volume.disk_id);
        let Some(disk) = disks.next() else {
            return Err(invalid("named volume references a missing disk"));
        };
        if disks.next().is_some()
            || disk.read_only != (volume.access_mode == VolumeAccessMode::ReadOnly)
        {
            return Err(invalid("named volume disk identity or mode mismatch"));
        }
    }
    Ok(())
}

/// Checks named attachments against canonical platform filesystem mount paths.
///
/// A platform mount at `/` conflicts with every named attachment.
///
/// # Errors
/// Rejects ambiguous path text, excessive path lengths, and overlapping mounts.
pub fn validate_guest_volume_mounts(
    volumes: &[VmGuestVolume],
    mounts: &[&str],
) -> Result<(), VmError> {
    for path in mounts {
        if !path.starts_with('/')
            || path.len() > volume_domain::MAX_GUEST_MOUNT_PATH_BYTES
            || path
                .chars()
                .any(|character| character.is_control() || character == '\\')
            || (*path != "/"
                && path[1..]
                    .split('/')
                    .any(|part| matches!(part, "" | "." | "..")))
        {
            return Err(invalid("platform guest mount path is not canonical"));
        }
        if volumes
            .iter()
            .any(|volume| paths_overlap(volume.guest_path.as_str(), path))
        {
            return Err(invalid(
                "named volume conflicts with a guest filesystem mount",
            ));
        }
    }
    Ok(())
}

fn paths_overlap(left: &str, right: &str) -> bool {
    left == "/"
        || right == "/"
        || left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn invalid(reason: &str) -> VmError {
    VmError::InvalidSpec {
        field: "guest_volumes".to_owned(),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
#[path = "guest_volume_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "guest_initialization_tests.rs"]
mod initialization_tests;
