use std::{
    fs,
    path::{Path, PathBuf},
};
use vm_trait::{RootFilesystem, VmSpec};

pub enum PathSnapshot {
    File { path: PathBuf, bytes: Vec<u8> },
    Directory(PathBuf),
}

impl PathSnapshot {
    fn capture(path: &Path) -> Self {
        if path.is_dir() {
            Self::Directory(path.to_path_buf())
        } else {
            Self::File {
                path: path.to_path_buf(),
                bytes: fs::read(path).expect("read caller-owned fixture"),
            }
        }
    }

    pub(crate) fn assert_unchanged(self) {
        match self {
            Self::File { path, bytes } => {
                assert_eq!(
                    fs::read(&path).expect("caller-owned file survives"),
                    bytes,
                    "provider modified caller-owned file {}",
                    path.display()
                );
            }
            Self::Directory(path) => {
                assert!(
                    path.is_dir(),
                    "provider removed caller-owned directory {}",
                    path.display()
                );
            }
        }
    }
}

pub fn snapshots(spec: &VmSpec) -> Vec<PathSnapshot> {
    let mut snapshots = Vec::with_capacity(1 + spec.disks.len() + spec.mounts.len());
    match &spec.root {
        RootFilesystem::Directory { host_path } | RootFilesystem::Disk { host_path, .. } => {
            snapshots.push(PathSnapshot::capture(host_path));
        }
        _ => {}
    }
    snapshots.extend(
        spec.disks
            .iter()
            .map(|disk| PathSnapshot::capture(&disk.host_path)),
    );
    snapshots.extend(
        spec.mounts
            .iter()
            .map(|mount| PathSnapshot::capture(&mount.host_path)),
    );
    snapshots
}
