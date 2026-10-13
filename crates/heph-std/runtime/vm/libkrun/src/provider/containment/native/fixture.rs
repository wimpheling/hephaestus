//! Original native fixture roots, actual Child/pidfd death observations.
use super::{HELPER, ID};
use crate::{LibkrunConfig, cgroup::Cgroup};
use rustix::process::{Signal, pidfd_send_signal};
use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use vm_trait::VmProviderOwnerScope;

pub(super) struct FixtureRoot {
    pub(super) directory: Option<tempfile::TempDir>,
    pub(super) held: AtomicBool,
}
impl FixtureRoot {
    pub(super) fn path(&self) -> &std::path::Path {
        self.directory.as_ref().unwrap().path()
    }
    fn uncertain(&self, error: &io::Error) {
        self.held.store(true, Ordering::Relaxed);
        eprintln!(
            "UNCERTAIN fixture process death; preserving {}: {error}",
            self.path().display()
        );
    }
}
impl Drop for FixtureRoot {
    fn drop(&mut self) {
        if self.held.load(Ordering::Relaxed) {
            let retained = self.directory.take().unwrap().keep();
            eprintln!("HELD native fixture roots retained: {}", retained.display());
        }
    }
}

pub(super) struct Supervisor(pub(super) Child, Arc<FixtureRoot>);
impl Drop for Supervisor {
    fn drop(&mut self) {
        let _kill = self.0.kill();
        if let Err(error) = wait_child(&mut self.0) {
            self.1.uncertain(&error);
        }
    }
}

pub(super) struct OriginalWorker(pub(super) OwnedFd, pub(super) Arc<FixtureRoot>);
impl Drop for OriginalWorker {
    fn drop(&mut self) {
        let _kill = pidfd_send_signal(&self.0, Signal::KILL);
        if let Err(error) = wait_kernel_death(&self.0) {
            self.1.uncertain(&error);
        }
    }
}

pub(super) fn spawn_supervisor(
    runtime: &std::path::Path,
    scope: &VmProviderOwnerScope,
    ready: &std::path::Path,
    root: &Arc<FixtureRoot>,
) -> Supervisor {
    Supervisor(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", HELPER, "--ignored", "--nocapture"])
            .env("HEPH_CONTAINMENT_RUNTIME", runtime)
            .env("HEPH_CONTAINMENT_NAMESPACE", scope.namespace())
            .env("HEPH_CONTAINMENT_READY", ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
        Arc::clone(root),
    )
}

pub(super) fn configuration_for_parent(runtime: PathBuf) -> LibkrunConfig {
    let root = PathBuf::from(
        std::env::var_os("HEPHAESTUS_LIBKRUN_OWNED_ROOTFS").expect("qualified image root"),
    );
    let cgroup = PathBuf::from(
        std::env::var_os("HEPHAESTUS_LIBKRUN_OWNED_CGROUP_ROOT")
            .expect("actual dedicated delegated cgroup"),
    );
    let worker = PathBuf::from(
        std::env::var_os("HEPHAESTUS_LIBKRUN_CONTAINMENT_WORKER")
            .expect("actual compiled worker binary"),
    );
    LibkrunConfig::new(
        runtime,
        vec![root.clone()],
        vec![root.clone()],
        vec![root],
        worker,
        cgroup,
    )
}

// A test-owned process proves actual cgroup termination independently of the
// stopped pre-placement worker's gate; it is not a fabricated guest/passt proof.
pub(super) fn placed_control(config: &LibkrunConfig, root: &Arc<FixtureRoot>) -> Supervisor {
    let child = Supervisor(
        Command::new("/bin/sleep").arg("300").spawn().unwrap(),
        Arc::clone(root),
    );
    Cgroup::existing(config, ID)
        .add_process(child.0.id())
        .unwrap();
    child
}

pub(super) fn assert_placed_stopped(placed: &mut Supervisor, config: &LibkrunConfig) {
    use std::os::unix::process::ExitStatusExt as _;
    let status =
        wait_child(&mut placed.0).expect("cgroup must terminate actual placed Child within bound");
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert!(!config.cgroup_root.join(ID).exists());
}

pub(super) fn wait_child(child: &mut Child) -> io::Result<ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "original Child exit unconfirmed",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// rustix has no enabled event feature; libc poll observes only this retained
// pidfd. This test-only ABI boundary does not reopen a PID or reap an orphan.
#[allow(unsafe_code)]
pub(super) fn wait_kernel_death(original: &OwnedFd) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut descriptor = libc::pollfd {
        fd: original.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: one initialized pollfd remains valid, and the borrowed original
        // OwnedFd remains live throughout this zero-timeout, one-element call.
        let count = unsafe { libc::poll(&raw mut descriptor, 1, 0) };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        } else if descriptor.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(io::Error::other("original pidfd readiness invalid"));
        } else if descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "original pidfd kernel death unconfirmed",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
