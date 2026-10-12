//! Real exec/descriptor transfer tests without KVM or native passt claims.
use super::{inherited_runtime_directory, parse_launch_arguments};
use crate::{directory::ChildDirectory, protocol::SUPERVISOR_SOCKET_NAME};
use std::{
    fs,
    io::Write,
    os::unix::{
        fs::MetadataExt,
        net::{UnixListener, UnixStream},
    },
    process::{Command, Stdio},
};

#[test]
fn runtime_stdin_inheritance_keeps_worker_and_descendant_in_original_directory() {
    let temp = tempfile::tempdir().unwrap();
    let directory = ChildDirectory::create(
        temp.path(),
        "vm",
        rustix::fs::Mode::from_bits_truncate(0o700),
        |_| {},
    )
    .unwrap();
    let listener =
        UnixListener::bind(directory.path().unwrap().join(SUPERVISOR_SOCKET_NAME)).unwrap();
    let original = temp.path().join("original");
    fs::rename(temp.path().join("vm"), &original).unwrap();
    fs::create_dir(temp.path().join("vm")).unwrap();
    fs::write(temp.path().join("vm/sentinel"), b"foreign").unwrap();
    // An unrelated private descriptor must not cross exec alongside FD0.
    let private = fs::File::create(temp.path().join("private-lock")).unwrap();
    let identity = private.metadata().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "worker::path_tests::runtime_descriptor_subprocess",
            "--nocapture",
        ])
        .env("HEPH_RUNTIME_DESCRIPTOR_CHILD", "worker")
        .env(
            "HEPH_UNRELATED_DESCRIPTOR_DEVICE",
            identity.dev().to_string(),
        )
        .env(
            "HEPH_UNRELATED_DESCRIPTOR_INODE",
            identity.ino().to_string(),
        )
        .stdin(Stdio::from(directory.file().unwrap().try_clone().unwrap()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(original.join("worker-owned")).unwrap(), b"worker");
    assert_eq!(
        fs::read(original.join("descendant-owned")).unwrap(),
        b"descendant"
    );
    let (mut stream, _) = listener.accept().unwrap();
    let mut bytes = [0_u8; 6];
    std::io::Read::read_exact(&mut stream, &mut bytes).unwrap();
    assert_eq!(&bytes, b"worker");
    assert_eq!(
        fs::read(temp.path().join("vm/sentinel")).unwrap(),
        b"foreign"
    );
    assert!(!temp.path().join("vm/worker-owned").exists());
    assert!(!temp.path().join("vm/descendant-owned").exists());
    drop(private);
}

#[test]
fn runtime_descriptor_subprocess() {
    if std::env::var("HEPH_RUNTIME_DESCRIPTOR_CHILD").as_deref() != Ok("worker") {
        return;
    }
    let (socket, runtime) =
        parse_launch_arguments(["worker".into(), "--runtime-stdin".into()].into_iter()).unwrap();
    let runtime = runtime.unwrap();
    assert!(
        rustix::io::fcntl_getfd(&*runtime)
            .unwrap()
            .contains(rustix::io::FdFlags::CLOEXEC)
    );
    let device: u64 = std::env::var("HEPH_UNRELATED_DESCRIPTOR_DEVICE")
        .unwrap()
        .parse()
        .unwrap();
    let inode: u64 = std::env::var("HEPH_UNRELATED_DESCRIPTOR_INODE")
        .unwrap()
        .parse()
        .unwrap();
    for entry in fs::read_dir("/proc/self/fd").unwrap() {
        if let Ok(metadata) = fs::metadata(entry.unwrap().path()) {
            assert!(
                metadata.dev() != device || metadata.ino() != inode,
                "unrelated private descriptor crossed exec"
            );
        }
    }
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.write_all(b"worker").unwrap();
    fs::write(
        crate::directory::fd_path(&runtime).join("worker-owned"),
        b"worker",
    )
    .unwrap();
    // The same safe FD0 transfer supplies descendant-local /proc paths, which
    // production passt receives. This child is an exec fixture, not passt.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "worker::path_tests::runtime_descriptor_descendant",
            "--nocapture",
        ])
        .env("HEPH_RUNTIME_DESCRIPTOR_CHILD", "descendant")
        .stdin(Stdio::from(runtime.try_clone().unwrap()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn runtime_descriptor_descendant() {
    if std::env::var("HEPH_RUNTIME_DESCRIPTOR_CHILD").as_deref() != Ok("descendant") {
        return;
    }
    let directory = inherited_runtime_directory().unwrap();
    fs::write("/proc/self/fd/0/./descendant-owned", b"descendant").unwrap();
    assert_eq!(
        fs::read(crate::directory::fd_path(&directory).join("descendant-owned")).unwrap(),
        b"descendant"
    );
}

#[test]
fn legacy_worker_socket_argument_is_preserved_without_descriptor_adoption() {
    let (socket, runtime) = parse_launch_arguments(
        ["worker".into(), "--socket".into(), "/legacy/socket".into()].into_iter(),
    )
    .unwrap();
    assert_eq!(socket, std::path::PathBuf::from("/legacy/socket"));
    assert!(runtime.is_none());
}

#[test]
fn runtime_stdin_rejects_regular_files_and_nonprivate_directories() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("file"), b"not a directory").unwrap();
    fs::create_dir(temp.path().join("directory")).unwrap();
    fs::set_permissions(
        temp.path().join("directory"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    for name in ["file", "directory"] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "worker::path_tests::invalid_runtime_descriptor_subprocess",
                "--nocapture",
            ])
            .env("HEPH_RUNTIME_DESCRIPTOR_CHILD", "reject")
            .stdin(Stdio::from(fs::File::open(temp.path().join(name)).unwrap()))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn invalid_runtime_descriptor_subprocess() {
    if std::env::var("HEPH_RUNTIME_DESCRIPTOR_CHILD").as_deref() != Ok("reject") {
        return;
    }
    assert!(inherited_runtime_directory().is_err());
}
