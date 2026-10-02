use crate::validation::PreparedSpec;
use std::io;

pub fn named_volumes(spec: &PreparedSpec) -> io::Result<Vec<crate::protocol::GuestVolume>> {
    vm_trait::validate_guest_volume_set(&spec.guest_volumes).map_err(io::Error::other)?;
    if !spec.guest_volumes.is_empty()
        && spec.labels.keys().any(|key| {
            matches!(
                key.as_str(),
                "hephaestus.agent-state.filesystem-uuid"
                    | "hephaestus.agent-state.mount-path"
                    | "hephaestus.oci-scratch.filesystem-uuid"
                    | "hephaestus.oci-scratch.mount-path"
            )
        })
    {
        return Err(io::Error::other(
            "named and legacy volume metadata conflict",
        ));
    }
    let paths = spec
        .mounts
        .iter()
        .map(|mount| {
            mount
                .guest_path
                .to_str()
                .ok_or_else(|| io::Error::other("platform mount path is not UTF-8"))
        })
        .collect::<io::Result<Vec<_>>>()?;
    vm_trait::validate_guest_volume_mounts(&spec.guest_volumes, &paths)
        .map_err(io::Error::other)?;
    for volume in &spec.guest_volumes {
        let mut disks = spec.disks.iter().filter(|disk| disk.id == volume.disk_id());
        let disk = disks
            .next()
            .ok_or_else(|| io::Error::other("named guest disk unavailable"))?;
        if disks.next().is_some()
            || disk.read_only != (volume.access_mode() == vm_trait::VolumeAccessMode::ReadOnly)
        {
            return Err(io::Error::other(
                "named guest disk identity or access mismatch",
            ));
        }
    }
    crate::validation::validate_named_disk_files(&spec.guest_volumes, &spec.disks)
        .map_err(io::Error::other)?;
    Ok(spec.guest_volumes.clone())
}
