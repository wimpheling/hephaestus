use rustix::fs::{Mode, OFlags, fstat, mkdirat, open, openat};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    os::fd::{AsRawFd, OwnedFd},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use vm_libkrun::protocol::{GuestMount, GuestStateVolume, GuestVolume};
use vm_trait::{VmVolumeInitializationPurpose, VolumeAccessMode};

use super::mounts::{find_ext4_device_in, mount_ext4_access, unmount};
use crate::{AGENT_GID, AGENT_UID};

/// Validates the entire bootstrap attachment graph before the first mount.
pub fn validate_boot_volumes(
    volumes: &[GuestVolume],
    legacy: Option<&GuestStateVolume>,
    mounts: &[GuestMount],
) -> io::Result<()> {
    vm_trait::validate_guest_volume_set(volumes).map_err(io::Error::other)?;
    if legacy.is_some() && !volumes.is_empty() {
        return Err(invalid("named and legacy volumes cannot combine"));
    }
    let paths = mounts
        .iter()
        .map(|mount| {
            mount
                .guest_path
                .to_str()
                .ok_or_else(|| invalid("platform mount path is not UTF-8"))
        })
        .collect::<io::Result<Vec<_>>>()?;
    vm_trait::validate_guest_volume_mounts(volumes, &paths).map_err(io::Error::other)?;
    if let Some(legacy) = legacy {
        if !matches!(
            legacy.guest_path.to_str(),
            Some("/var/lib/hephaestus" | "/workspace/buildah")
        ) || uuid::Uuid::parse_str(&legacy.filesystem_uuid).is_err()
            || mounts.iter().any(|mount| {
                legacy.guest_path.starts_with(&mount.guest_path)
                    || mount.guest_path.starts_with(&legacy.guest_path)
            })
        {
            return Err(invalid("legacy volume path, identity, or mount conflict"));
        }
    }
    Ok(())
}

/// Creates directories relative to pinned descriptors without following links.
/// The root parameter is `/` in the guest and isolated temporary storage in tests.
pub fn secure_mountpoint(root: &Path, path: &Path) -> io::Result<OwnedFd> {
    walk_mountpoint(root, path, true, false)
}

fn walk_mountpoint(root: &Path, path: &Path, create: bool, stable: bool) -> io::Result<OwnedFd> {
    let text = path
        .to_str()
        .ok_or_else(|| invalid("mount path is not UTF-8"))?;
    if !text.starts_with('/')
        || text.len() > vm_trait::MAX_GUEST_MOUNT_PATH_BYTES
        || text
            .chars()
            .any(|value| value.is_control() || value == '\\')
        || text[1..]
            .split('/')
            .any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(invalid("mount path is not canonical"));
    }
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = open(root, flags, Mode::empty())?;
    for component in text[1..].split('/') {
        if stable {
            let stat = fstat(&directory)?;
            verify_stable_ancestor(stat.st_uid, stat.st_mode)?;
        }
        if create {
            match mkdirat(&directory, component, Mode::from_bits_truncate(0o755)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(error) => return Err(error.into()),
            }
        }
        directory = openat(&directory, component, flags, Mode::empty())?;
    }
    Ok(directory)
}

pub fn pinned_path(directory: &OwnedFd) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
}

pub fn mount_named_volumes(volumes: &[GuestVolume]) -> io::Result<MountedVolumes> {
    let mut mounted = MountedVolumes::default();
    for volume in volumes {
        if let Err(error) = mount_named_volume(volume) {
            return match mounted.unmount_all() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(io::Error::other(format!(
                    "{error}; volume cleanup failed: {cleanup}"
                ))),
            };
        }
        mounted.push(PathBuf::from(volume.guest_path().as_str()))?;
    }
    Ok(mounted)
}

fn mount_named_volume(volume: &GuestVolume) -> io::Result<()> {
    let path = Path::new(volume.guest_path().as_str());
    let device = find_ext4_device_in(
        volume.filesystem_uuid(),
        Path::new("/sys/class/block"),
        Path::new("/dev"),
    )?;
    let name = device
        .file_name()
        .ok_or_else(|| invalid("block device name unavailable"))?;
    let read_only = volume.access_mode() == VolumeAccessMode::ReadOnly;
    let device_mode = fs::read_to_string(Path::new("/sys/class/block").join(name).join("ro"))?;
    verify_device_mode(&device_mode, read_only)?;
    if read_only {
        verify_clean_ext4(&device)?;
    }
    let directory = walk_mountpoint(Path::new("/"), path, true, true)?;
    mount_ext4_access(&device, &pinned_path(&directory), read_only)?;
    drop(directory);
    if let Err(error) =
        prepare_mounted_volume(path, &device, read_only, volume.initialization_purpose())
    {
        return match unmount(path) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(io::Error::other(format!(
                "{error}; volume cleanup failed: {cleanup}"
            ))),
        };
    }
    Ok(())
}

/// Reopens the mounted filesystem through stable, non-symlink ancestors.
pub fn prepare_mounted_volume(
    path: &Path,
    device: &Path,
    read_only: bool,
    purpose: VmVolumeInitializationPurpose,
) -> io::Result<()> {
    let directory = pin_mounted_volume(path, device)?;
    if purpose == VmVolumeInitializationPurpose::BuiltinStateSQLite {
        if read_only || path != Path::new("/var/lib/hephaestus") {
            return Err(invalid(
                "built-in initialization requires exact writable state mount",
            ));
        }
        super::builtin_state::initialize_pinned(&directory)?;
    } else if !read_only {
        rustix::fs::fchown(
            &directory,
            Some(rustix::fs::Uid::from_raw(AGENT_UID)),
            Some(rustix::fs::Gid::from_raw(AGENT_GID)),
        )?;
    }
    Ok(())
}

/// Pins and checks the post-mount device before any ownership or database IO.
pub fn pin_mounted_volume(path: &Path, device: &Path) -> io::Result<OwnedFd> {
    let directory = walk_mountpoint(Path::new("/"), path, false, true)?;
    let identity = mounted_identity(&directory)?;
    if identity.0 != fs::symlink_metadata(device)?.rdev() {
        return Err(invalid(
            "mounted filesystem differs from selected block device",
        ));
    }
    Ok(directory)
}

pub fn verify_device_mode(mode: &str, read_only: bool) -> io::Result<()> {
    if mode.trim() != if read_only { "1" } else { "0" } {
        return Err(invalid(
            "kernel block device access differs from declared volume access",
        ));
    }
    Ok(())
}

fn verify_clean_ext4(device: &Path) -> io::Result<()> {
    let mut file = File::open(device)?;
    file.seek(SeekFrom::Start(1024))?;
    let mut superblock = [0_u8; 104];
    file.read_exact(&mut superblock)?;
    verify_clean_superblock(&superblock)
}

/// Rejects known dirty/error/recovery state; the kernel performs the normal
/// read-only mount validation. No journal suppression option is used.
pub fn verify_clean_superblock(superblock: &[u8; 104]) -> io::Result<()> {
    let state = u16::from_le_bytes([superblock[58], superblock[59]]);
    let incompatible = u32::from_le_bytes(superblock[96..100].try_into().expect("fixed slice"));
    if superblock[56..58] != [0x53, 0xef]
        || state & 1 == 0
        || state & 2 != 0
        || incompatible & 4 != 0
    {
        return Err(invalid(
            "read-only ext4 volume requires clean filesystem preparation",
        ));
    }
    Ok(())
}

#[derive(Default)]
pub struct MountedVolumes {
    paths: Vec<PathBuf>,
    devices: BTreeMap<PathBuf, (u64, u64)>,
}

impl MountedVolumes {
    pub fn push(&mut self, path: PathBuf) -> io::Result<()> {
        let identity = (|| {
            let descriptor = walk_mountpoint(Path::new("/"), &path, false, true)?;
            mounted_identity(&descriptor)
        })();
        let identity = match identity {
            Ok(identity) => identity,
            Err(error) => {
                // The workload has not started when an attachment is recorded.
                return match unmount(&path) {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(io::Error::other(format!(
                        "{error}; volume cleanup failed: {cleanup}"
                    ))),
                };
            }
        };
        self.devices.insert(path.clone(), identity);
        self.paths.push(path);
        Ok(())
    }

    /// Attempts every unmount in reverse order. Failure remains visible and
    /// must cause host-side VM destruction before any attachment is released.
    pub fn unmount_all(&mut self) -> io::Result<()> {
        unmount_reverse(&mut self.paths, |path| {
            let descriptor = walk_mountpoint(Path::new("/"), path, false, true)?;
            if self.devices.get(path) != Some(&mounted_identity(&descriptor)?) {
                return Err(invalid(
                    "mounted filesystem identity changed before cleanup",
                ));
            }
            // Stable ancestors prevent the guest user from replacing this path.
            // Close the descriptor before unmount so it does not keep it busy.
            drop(descriptor);
            unmount(path)
        })
    }
}

impl Drop for MountedVolumes {
    fn drop(&mut self) {
        if let Err(error) = self.unmount_all() {
            let _diagnostic = writeln!(io::stderr(), "guest volume cleanup failed: {error}");
        }
    }
}

pub fn unmount_reverse(
    paths: &mut Vec<PathBuf>,
    mut remove: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let mut failed = Vec::new();
    let mut first_error = None;
    while let Some(path) = paths.pop() {
        if let Err(error) = remove(&path) {
            failed.push(path);
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    failed.reverse();
    *paths = failed;
    first_error.map_or(Ok(()), Err)
}

pub fn mounted_identity(descriptor: &OwnedFd) -> io::Result<(u64, u64)> {
    let file = File::open(format!("/proc/self/fdinfo/{}", descriptor.as_raw_fd()))?;
    let mut details = String::new();
    file.take(4096).read_to_string(&mut details)?;
    let mount_id = details
        .lines()
        .find_map(|line| line.strip_prefix("mnt_id:")?.trim().parse::<u64>().ok())
        .ok_or_else(|| invalid("kernel mount identity unavailable"))?;
    Ok((fstat(descriptor)?.st_dev, mount_id))
}

fn verify_stable_ancestor(uid: u32, mode: u32) -> io::Result<()> {
    if uid == AGENT_UID || mode & 0o022 != 0 {
        return Err(invalid(
            "mount ancestor can be replaced by the guest workload",
        ));
    }
    Ok(())
}

fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}

#[cfg(test)]
#[path = "volume_mount_tests.rs"]
mod tests;
