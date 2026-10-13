use super::{DATABASE, SIDECARS, pin_child_directory, prepare_database, verify_mount};
use crate::volume_mounts::mounted_identity;
use rusqlite::Connection;
use rustix::fs::{Mode, OFlags, fstat, mkfifoat, open};
use std::{
    fs,
    os::{fd::OwnedFd, unix::fs::symlink},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const FIXTURE_MOUNT: &str = "HEPH_BUILTIN_STATE_FIXTURE_MOUNT";

// Runs only in a child whose cwd was pinned before exec. The parent tests never
// change process-global cwd, and this fixture does not require guest root UID.
#[test]
fn subprocess_helper() {
    let Ok(expected) = std::env::var(FIXTURE_MOUNT) else {
        return;
    };
    let (device, mount) = expected.split_once(':').unwrap();
    let directory = open(
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    verify_mount(
        &directory,
        (device.parse().unwrap(), mount.parse().unwrap()),
    )
    .unwrap();
    let stat = fstat(&directory).unwrap();
    prepare_database(&directory, stat.st_uid, stat.st_gid).unwrap();
}

fn initialize(path: &Path, wrong_mount: bool) -> bool {
    let directory = open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    initialize_descriptor(&directory, wrong_mount)
}

fn initialize_descriptor(directory: &OwnedFd, wrong_mount: bool) -> bool {
    let (device, mount) = mounted_identity(directory).unwrap();
    let expected_mount = if wrong_mount { mount + 1 } else { mount };
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "builtin_state::tests::subprocess_helper"])
        .env(FIXTURE_MOUNT, format!("{device}:{expected_mount}"));
    pin_child_directory(&mut command, directory);
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.success();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("pinned database helper did not reject or complete promptly");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn child_uses_pinned_directory_after_lexical_root_replacement() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("mounted");
    let retained = root.path().join("retained");
    fs::create_dir(&path).unwrap();
    let directory = open(
        &path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    fs::rename(&path, &retained).unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(path.join(DATABASE), b"foreign replacement").unwrap();
    assert!(initialize_descriptor(&directory, false));
    assert_eq!(
        fs::read(path.join(DATABASE)).unwrap(),
        b"foreign replacement"
    );
    assert!(retained.join(DATABASE).is_file());
}

#[test]
fn fresh_database_uses_wal_and_survives_reopen() {
    let root = TempDir::new().unwrap();
    assert!(initialize(root.path(), false));
    let database = root.path().join(DATABASE);
    let connection = Connection::open(&database).unwrap();
    let mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    connection
        .execute_batch(
            "CREATE TABLE workload(value TEXT); INSERT INTO workload VALUES ('retained');",
        )
        .unwrap();
    drop(connection);
    assert!(initialize(root.path(), false));
    let connection = Connection::open(database).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM workload", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "retained");
}

#[test]
fn existing_schema_journal_mode_and_bytes_are_preserved() {
    let root = TempDir::new().unwrap();
    let database = root.path().join(DATABASE);
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE custom_schema(value INTEGER); INSERT INTO custom_schema VALUES (42);",
        )
        .unwrap();
    drop(connection);
    let before = fs::read(&database).unwrap();
    assert!(initialize(root.path(), false));
    assert_eq!(fs::read(&database).unwrap(), before);
    let connection = Connection::open(database).unwrap();
    let mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "delete");
    let value: i64 = connection
        .query_row("SELECT value FROM custom_schema", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, 42);
}

#[test]
fn corrupt_empty_and_orphan_files_are_not_initialized() {
    for bytes in [&b"not a database"[..], &b""[..]] {
        let root = TempDir::new().unwrap();
        let database = root.path().join(DATABASE);
        fs::write(&database, bytes).unwrap();
        assert!(!initialize(root.path(), false));
        assert_eq!(fs::read(database).unwrap(), bytes);
    }
    for sidecar in SIDECARS {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join(sidecar), b"orphan evidence").unwrap();
        assert!(!initialize(root.path(), false));
        assert!(!root.path().join(DATABASE).exists());
        assert_eq!(
            fs::read(root.path().join(sidecar)).unwrap(),
            b"orphan evidence"
        );
    }
}

#[test]
fn symlinks_and_linked_inodes_leave_foreign_bytes_untouched() {
    for name in std::iter::once(DATABASE).chain(SIDECARS) {
        let root = TempDir::new().unwrap();
        let foreign = TempDir::new().unwrap();
        let foreign_file = foreign.path().join("foreign-file");
        fs::write(&foreign_file, b"foreign bytes").unwrap();
        symlink(&foreign_file, root.path().join(name)).unwrap();
        assert!(!initialize(root.path(), false));
        assert_eq!(fs::read(foreign_file).unwrap(), b"foreign bytes");
        if name != DATABASE {
            assert!(!root.path().join(DATABASE).exists());
        }
    }
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("foreign"), b"linked evidence").unwrap();
    fs::hard_link(root.path().join("foreign"), root.path().join(DATABASE)).unwrap();
    assert!(!initialize(root.path(), false));
    assert_eq!(
        fs::read(root.path().join("foreign")).unwrap(),
        b"linked evidence"
    );
}

#[test]
fn wrong_pinned_mount_fails_before_creating_database() {
    let root = TempDir::new().unwrap();
    assert!(!initialize(root.path(), true));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn fifo_database_and_sidecars_are_rejected_promptly_without_sqlite_io() {
    for name in std::iter::once(DATABASE).chain(SIDECARS) {
        let root = TempDir::new().unwrap();
        let directory = open(
            root.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        mkfifoat(&directory, name, Mode::from_bits_truncate(0o600)).unwrap();
        drop(directory);
        assert!(!initialize(root.path(), false));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
