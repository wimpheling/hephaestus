//! Real subprocess/OFD controls and source-owned filesystem negatives, no VMM claim.
#[path = "record_tests.rs"]
mod records;

use super::{birth::filename, *};
use crate::{cgroup::Cgroup, directory::ChildDirectory, provider::ownership::ProviderOwner};
use std::{
    fs,
    process::{Child, Command},
    sync::Arc,
};
use tempfile::TempDir;
use vm_trait::{VmId, VmProviderOwnerScope};

struct Fixture {
    _directory: TempDir,
    config: crate::LibkrunConfig,
    owner: ProviderOwner,
    scope: VmProviderOwnerScope,
    id: VmId,
    runtime: ChildDirectory,
    cgroup: Cgroup,
    record: Arc<BirthRecord>,
}

impl Fixture {
    fn new() -> Self {
        let directory = TempDir::new().unwrap();
        let runtime_root = directory.path().join("runtime");
        let cgroup_root = directory.path().join("cgroup");
        fs::create_dir(&runtime_root).unwrap();
        fs::create_dir(&cgroup_root).unwrap();
        let mut config = crate::LibkrunConfig::new(
            &runtime_root,
            vec![],
            vec![],
            vec![],
            "/bin/true",
            &cgroup_root,
        );
        config.enforce_cgroup_v2 = false;
        let scope =
            VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "containment-test".into())
                .unwrap();
        let owner = ProviderOwner::bootstrap(&config, &scope).unwrap();
        let id = VmId("birth".into());
        let runtime = ChildDirectory::create(
            &runtime_root,
            &id.0,
            Mode::from_bits_truncate(0o700),
            |_| {},
        )
        .unwrap();
        let cgroup = Cgroup::create_tracked(&config, &id.0, |_| {}).unwrap();
        let guard = owner.validate(&config).unwrap();
        let mut record = None;
        BirthRecord::publish(
            (&guard.runtime, &guard.cgroup),
            &scope,
            &id,
            (
                &*runtime.file().unwrap(),
                &*cgroup.allocated_directory().unwrap().file().unwrap(),
            ),
            |value| record = Some(value),
        )
        .unwrap();
        drop(guard);
        Self {
            _directory: directory,
            config,
            owner,
            scope,
            id,
            runtime,
            cgroup,
            record: record.unwrap(),
        }
    }

    fn recover(&self) -> Result<Option<File>, VmError> {
        self.owner.require_containment()?;
        let guard = self.owner.validate(&self.config)?;
        let gate = recover(&self.config, &guard, &self.scope, &self.id)?;
        self.owner.validate_guard(&self.config, &guard)?;
        Ok(gate)
    }

    fn record_path(&self) -> std::path::PathBuf {
        self.config.runtime_root.join(filename(&self.id))
    }
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _kill = self.0.kill();
        let _wait = self.0.wait();
    }
}

#[test]
fn inherited_gate_is_separate_from_retained_parent_directory_custody() {
    let fixture = Fixture::new();
    let directory = fixture.runtime.file().unwrap();
    let mut command = Command::new("/bin/sleep");
    command.arg("60").stdin(worker_stdin(&directory).unwrap());
    let mut child = Process(command.spawn().unwrap());
    drop(command);
    assert!(
        exclusive(&directory).is_err(),
        "actual child FD0 owns the shared gate"
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let gate = exclusive(&directory).unwrap();
    drop(gate);
    assert!(
        directory.metadata().unwrap().is_dir(),
        "retained parent custody is still live but owns no gate"
    );
}

#[test]
fn failed_spawn_releases_only_parent_gate_by_dropping_command() {
    let fixture = Fixture::new();
    let directory = fixture.runtime.file().unwrap();
    let mut command = Command::new("/definitely-missing-heph-containment-worker");
    command.stdin(worker_stdin(&directory).unwrap());
    assert!(
        exclusive(&directory).is_err(),
        "the configured Command owns its Stdio before spawn"
    );
    assert!(command.spawn().is_err());
    drop(command);
    assert!(exclusive(&directory).is_ok());
}

#[test]
fn surviving_unplaced_child_holds_recovery_then_actual_exit_permits_original_retry() {
    let fixture = Fixture::new();
    let directory = fixture.runtime.file().unwrap();
    let mut command = Command::new("/bin/sleep");
    command.arg("60").stdin(worker_stdin(&directory).unwrap());
    let mut child = Process(command.spawn().unwrap());
    drop(command);
    assert!(fixture.recover().is_err());
    assert!(fixture.config.runtime_root.join(&fixture.id.0).exists());
    assert!(fixture.record_path().exists());
    // This controlled cgroup is emulated, and the actual subprocess was never
    // placed in it. Its removal cannot establish that subprocess's absence.
    assert!(!fixture.config.cgroup_root.join(&fixture.id.0).exists());
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    drop(fixture.recover().unwrap());
    assert!(!fixture.config.runtime_root.join(&fixture.id.0).exists());
    assert!(!fixture.record_path().exists());
    drop(fixture.recover().unwrap());
}
