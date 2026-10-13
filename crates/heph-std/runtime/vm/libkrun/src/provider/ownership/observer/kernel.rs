//! Read-only kernel filesystem/control qualification; fake directories never pass.
use super::super::{FileIdentity, files};
use rustix::fs::{Mode, OFlags};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, File},
    io::Read,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
};
use vm_trait::VmError;

const CONTROL_LIMIT: u64 = 4096;

pub(super) fn inspect(root: &File, original: &File) -> Result<BTreeMap<OsString, File>, VmError> {
    require_original_mount(root, original)?;
    let names = entries(root)?;
    let mut controls = BTreeMap::new();
    for name in names {
        let file = control(root, &name)?;
        controls.insert(name, file);
    }
    for name in ["cgroup.controllers", "cgroup.events"] {
        let file = controls
            .get(std::ffi::OsStr::new(name))
            .ok_or_else(|| files::invalid("required kernel cgroup control is missing"))?;
        required(std::ffi::OsStr::new(name), file)?;
    }
    Ok(controls)
}

pub(super) fn recheck(
    root: &File,
    original: &File,
    originals: &BTreeMap<OsString, File>,
) -> Result<(), VmError> {
    require_original_mount(root, original)?;
    if entries(root)? != originals.keys().cloned().collect() {
        return Err(files::invalid(
            "kernel cgroup entries changed during observation",
        ));
    }
    for (name, original) in originals {
        let current = control(root, name)?;
        if FileIdentity::read(&current)? != FileIdentity::read(original)? {
            return Err(files::invalid("kernel cgroup control inode changed"));
        }
        required(name, &current)?;
    }
    Ok(())
}

fn required(name: &std::ffi::OsStr, file: &File) -> Result<(), VmError> {
    if name == "cgroup.controllers" {
        controllers(&bounded(file)?)?;
    } else if name == "cgroup.events" {
        events(&bounded(file)?)?;
    }
    Ok(())
}

fn require_original_mount(root: &File, original: &File) -> Result<(), VmError> {
    require_kernel(root)?;
    require_kernel(original)?;
    if mount_id(root)? != mount_id(original)? {
        return Err(files::invalid(
            "original cgroup root mount identity changed",
        ));
    }
    Ok(())
}

fn require_kernel(file: &File) -> Result<(), VmError> {
    if rustix::fs::fstatfs(file).map_err(files::io_error)?.f_type != 0x6367_7270 {
        return Err(files::invalid(
            "absence observation requires actual cgroup-v2 filesystem",
        ));
    }
    Ok(())
}

fn entries(root: &File) -> Result<BTreeSet<OsString>, VmError> {
    fs::read_dir(format!("/proc/self/fd/{}", root.as_raw_fd()))
        .map_err(files::io_error)?
        .map(|entry| {
            let entry = entry.map_err(files::io_error)?;
            if !entry.file_type().map_err(files::io_error)?.is_file() {
                return Err(files::invalid(
                    "delegated cgroup contains child or noncontrol object",
                ));
            }
            Ok(entry.file_name())
        })
        .collect()
}

fn control(root: &File, name: &std::ffi::OsStr) -> Result<File, VmError> {
    // Other real kernel controls can be write-only: pin, never read/write them.
    let access = if name == "cgroup.controllers" || name == "cgroup.events" {
        OFlags::RDONLY | OFlags::NONBLOCK
    } else {
        OFlags::PATH
    };
    let file: File = rustix::fs::openat(
        root,
        name,
        access | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(files::io_error)?
    .into();
    require_kernel(&file)?;
    let metadata = file.metadata().map_err(files::io_error)?;
    let named = fs::symlink_metadata(
        std::path::PathBuf::from(format!("/proc/self/fd/{}", root.as_raw_fd())).join(name),
    )
    .map_err(files::io_error)?;
    if !metadata.is_file()
        || !named.is_file()
        || metadata.dev() != root.metadata().map_err(files::io_error)?.dev()
        || mount_id(&file)? != mount_id(root)?
        || metadata.dev() != named.dev()
        || metadata.ino() != named.ino()
    {
        return Err(files::invalid(
            "kernel cgroup control is redirected or uncertain",
        ));
    }
    Ok(file)
}

fn mount_id(file: &File) -> Result<u64, VmError> {
    let info =
        File::open(format!("/proc/self/fdinfo/{}", file.as_raw_fd())).map_err(files::io_error)?;
    let text = bounded(&info)?;
    let mut ids = text.lines().filter_map(|line| line.strip_prefix("mnt_id:"));
    let id = ids
        .next()
        .ok_or_else(|| files::invalid("kernel control mount identity is missing"))?
        .trim()
        .parse::<u64>()
        .map_err(files::io_error)?;
    if id == 0 || ids.next().is_some() {
        return Err(files::invalid("kernel control mount identity is ambiguous"));
    }
    Ok(id)
}

fn bounded(file: &File) -> Result<String, VmError> {
    let mut bytes = Vec::new();
    file.take(CONTROL_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(files::io_error)?;
    if bytes.len() > usize::try_from(CONTROL_LIMIT).expect("bounded control limit") {
        return Err(files::invalid(
            "kernel cgroup control exceeds bounded format",
        ));
    }
    String::from_utf8(bytes).map_err(files::io_error)
}

pub(super) fn controllers(text: &str) -> Result<(), VmError> {
    let mut seen = BTreeSet::new();
    for name in text.split_whitespace() {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            || !seen.insert(name)
            || seen.len() > 64
        {
            return Err(files::invalid("kernel cgroup controller list is malformed"));
        }
    }
    Ok(())
}

pub(super) fn events(text: &str) -> Result<(), VmError> {
    let mut seen = BTreeSet::new();
    for line in text.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2
            || !matches!(fields[0], "populated" | "frozen")
            || !matches!(fields[1], "0" | "1")
            || !seen.insert(fields[0])
        {
            return Err(files::invalid("kernel cgroup events are malformed"));
        }
    }
    if seen != BTreeSet::from(["populated", "frozen"]) {
        return Err(files::invalid("kernel cgroup events are incomplete"));
    }
    // Parent occupancy is not interpreted as host-wide orphan identity.
    Ok(())
}
