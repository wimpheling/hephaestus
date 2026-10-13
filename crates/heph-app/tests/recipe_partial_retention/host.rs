use super::fixture::BirthMode;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
use tempfile::TempDir;

pub type Tree = Vec<(PathBuf, u64, u64, u32, [u8; 32])>;
pub fn tools(tools: &TempDir, mode: BirthMode) {
    let mkfs = ["/usr/sbin/mkfs.ext4", "/sbin/mkfs.ext4"]
        .into_iter()
        .find(|path| Path::new(path).is_file())
        .expect("native e2fsprogs");
    let fsck = Path::new(mkfs).parent().unwrap().join("e2fsck");
    assert!(fsck.is_file());
    let body = if matches!(mode, BirthMode::FormatIncomplete) {
        "exit 1\n".to_owned()
    } else {
        format!("exec '{mkfs}' \"$@\"\n")
    };
    let script = format!(
        "#!/bin/sh\nprintf 'format\\n' >> '{}'\n{body}",
        tools.path().join("formats").display()
    );
    executable(&tools.path().join("mkfs.ext4"), &script);
    executable(
        &tools.path().join("e2fsck"),
        &format!("#!/bin/sh\nexec '{}' \"$@\"\n", fsck.display()),
    );
}
fn executable(path: &Path, bytes: &str) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
pub fn tree(root: &Path) -> Tree {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let stat = entry.metadata().unwrap();
            if stat.is_dir() {
                pending.push(path);
            } else {
                assert!(stat.is_file());
                files.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    stat.dev(),
                    stat.ino(),
                    stat.uid(),
                    Sha256::digest(fs::read(path).unwrap()).into(),
                ));
            }
        }
    }
    files.sort();
    files
}
