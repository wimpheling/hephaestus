use super::*;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
};
use tempfile::TempDir;

fn config(root: &Path) -> LibkrunConfig {
    let mut config = LibkrunConfig::new(
        root.join("runtime"),
        vec![root.into()],
        vec![root.into()],
        vec![root.into()],
        "/bin/true",
        root.join("cgroup"),
    );
    config.enforce_cgroup_v2 = false;
    config
}

fn fixture() -> (TempDir, LibkrunConfig) {
    let temp = TempDir::new().unwrap();
    let config = config(temp.path());
    fs::create_dir(&config.runtime_root).unwrap();
    fs::create_dir(&config.cgroup_root).unwrap();
    (temp, config)
}

struct ChildSession(Child);

impl Drop for ChildSession {
    fn drop(&mut self) {
        let _kill = self.0.kill();
        let _wait = self.0.wait();
    }
}

#[test]
fn child_session() {
    let Some(root) = std::env::var_os("HEPH_VM_OWNER_TEST_ROOT") else {
        return;
    };
    let owner = ProviderOwner::initialize(&config(Path::new(&root)), "test-host").unwrap();
    println!("supervisor-ready");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    let _read = std::io::stdin().read(&mut byte);
    drop(owner);
}

#[test]
fn separate_process_supervisor_blocks_takeover_until_death_then_same_namespace_reopens() {
    let (temp, config) = fixture();
    let mut child = ChildSession(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "provider::ownership::supervision_tests::child_session",
                "--nocapture",
            ])
            .env("HEPH_VM_OWNER_TEST_ROOT", temp.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut ready = false;
    for line in reader.by_ref().lines() {
        if line.unwrap().contains("supervisor-ready") {
            ready = true;
            break;
        }
    }
    assert!(ready, "child supervisor did not establish ownership");
    let root = files::open_root(&config.runtime_root, config.service_uid).unwrap();
    let expected = read_marker(&root).unwrap().unwrap().namespace;
    drop(root);
    assert!(ProviderOwner::initialize(&config, "test-host").is_err());
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    drop(child);
    let reopened = ProviderOwner::initialize(&config, "test-host").unwrap();
    assert_eq!(reopened.marker.namespace, expected);
    drop(reopened);
}

#[test]
fn exec_child_does_not_inherit_supervisor_lock() {
    let (_temp, config) = fixture();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    let expected = owner.scope().unwrap();
    let mut child = ChildSession(Command::new("/bin/sleep").arg("60").spawn().unwrap());
    assert!(child.0.try_wait().unwrap().is_none());
    drop(owner);
    let reopened = ProviderOwner::initialize(&config, "test-host").unwrap();
    assert_eq!(reopened.scope().unwrap(), expected);
    drop(reopened);
    drop(child);
}

#[test]
fn replaced_supervisor_lock_rejects_live_scope_and_restart_without_mutating_foreign_file() {
    let (_temp, config) = fixture();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    let lock = config.runtime_root.join(SUPERVISOR);
    fs::rename(&lock, config.runtime_root.join("old-supervisor-lock")).unwrap();
    fs::copy(config.runtime_root.join("old-supervisor-lock"), &lock).unwrap();
    assert!(owner.validate(&config).is_err());
    assert!(ProviderOwner::initialize(&config, "test-host").is_err());
    assert_eq!(fs::read(lock).unwrap(), Vec::<u8>::new());
    drop(owner);
}

#[test]
fn version_one_marker_is_not_adopted_or_given_new_lock_files() {
    let (_temp, config) = fixture();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    let path = config.runtime_root.join(MARKER);
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["version"] = 1.into();
    value.as_object_mut().unwrap().remove("supervisor");
    let historical = serde_json::to_vec(&value).unwrap();
    fs::write(&path, &historical).unwrap();
    drop(owner);
    fs::remove_file(config.runtime_root.join(SUPERVISOR)).unwrap();
    fs::remove_file(config.runtime_root.join(LOCK)).unwrap();
    let error = ProviderOwner::initialize(&config, "test-host")
        .err()
        .unwrap();
    assert!(error.to_string().contains("version 2 supervisor proof"));
    assert_eq!(fs::read(path).unwrap(), historical);
    assert!(!config.runtime_root.join(SUPERVISOR).exists());
    assert!(!config.runtime_root.join(LOCK).exists());
}

#[test]
fn redirected_or_missing_supervisor_lock_is_never_followed_or_recreated() {
    let (temp, config) = fixture();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    let path = config.runtime_root.join(SUPERVISOR);
    fs::rename(&path, config.runtime_root.join("pinned-old-lock")).unwrap();
    let foreign = temp.path().join("foreign");
    fs::write(&foreign, b"foreign bytes").unwrap();
    std::os::unix::fs::symlink(&foreign, &path).unwrap();
    assert!(owner.validate(&config).is_err());
    assert!(ProviderOwner::initialize(&config, "test-host").is_err());
    assert_eq!(fs::read(&foreign).unwrap(), b"foreign bytes");
    fs::remove_file(&path).unwrap();
    assert!(owner.validate(&config).is_err());
    assert!(ProviderOwner::initialize(&config, "test-host").is_err());
    assert!(!path.exists());
    drop(owner);
}
