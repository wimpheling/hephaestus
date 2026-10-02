use super::*;
use std::os::unix::fs::symlink;
use tempfile::TempDir;

#[test]
fn mountpoint_walk_never_follows_intermediate_or_final_symlinks() {
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    symlink(outside.path(), root.path().join("escape")).unwrap();
    assert!(secure_mountpoint(root.path(), Path::new("/escape/new")).is_err());
    assert!(!outside.path().join("new").exists());
    fs::create_dir(root.path().join("data")).unwrap();
    symlink(outside.path(), root.path().join("data/final")).unwrap();
    assert!(secure_mountpoint(root.path(), Path::new("/data/final")).is_err());
    for path in [
        "/",
        "/data//child",
        "/data/../child",
        "/data/./child",
        "/data/child/",
    ] {
        assert!(secure_mountpoint(root.path(), Path::new(path)).is_err());
    }
    let descriptor = secure_mountpoint(root.path(), Path::new("/safe/nested")).unwrap();
    assert!(root.path().join("safe/nested").is_dir());
    assert_eq!(
        fs::canonicalize(pinned_path(&descriptor)).unwrap(),
        root.path().join("safe/nested")
    );
}

#[test]
fn read_only_requires_kernel_device_mode_and_clean_ext4_without_journal_suppression() {
    verify_device_mode("1\n", true).unwrap();
    verify_device_mode("0\n", false).unwrap();
    for (value, mode) in [("0", true), ("1", false), ("unknown", true)] {
        assert!(verify_device_mode(value, mode).is_err());
    }
    let mut clean = [0_u8; 104];
    clean[56..58].copy_from_slice(&[0x53, 0xef]);
    clean[58] = 1;
    verify_clean_superblock(&clean).unwrap();
    for (offset, value) in [(58, 0), (58, 3), (96, 4), (56, 0)] {
        let mut dirty = clean;
        dirty[offset] = value;
        assert!(verify_clean_superblock(&dirty).is_err());
    }
    assert_ne!(
        super::super::mounts::ext4_mount_flags(true) & libc::MS_RDONLY,
        0
    );
    assert_eq!(
        super::super::mounts::ext4_mount_flags(false) & libc::MS_RDONLY,
        0
    );
}

#[test]
fn failed_unmount_does_not_stop_reverse_cleanup_or_erase_failed_evidence() {
    let mut paths = vec![
        PathBuf::from("/first"),
        PathBuf::from("/second"),
        PathBuf::from("/third"),
    ];
    let mut attempts = Vec::new();
    assert!(
        unmount_reverse(&mut paths, |path| {
            attempts.push(path.to_path_buf());
            if path == Path::new("/second") {
                Err(io::Error::other("busy"))
            } else {
                Ok(())
            }
        })
        .is_err()
    );
    assert_eq!(
        attempts,
        vec![
            PathBuf::from("/third"),
            PathBuf::from("/second"),
            PathBuf::from("/first")
        ]
    );
    assert_eq!(paths, vec![PathBuf::from("/second")]);
    unmount_reverse(&mut paths, |_| Ok(())).unwrap();
    assert!(paths.is_empty());
}

#[test]
fn guest_writable_ancestors_cannot_anchor_cleanup_paths() {
    verify_stable_ancestor(0, 0o755).unwrap();
    verify_stable_ancestor(1000, 0o755).unwrap();
    for (uid, mode) in [(AGENT_UID, 0o755), (0, 0o777), (0, 0o775)] {
        assert!(verify_stable_ancestor(uid, mode).is_err());
    }
}
