//! Actual Core calls through controlled ports; no SQL or native guest proof.
use super::Controlled;
use crate::{canonical::plural::Factory, support::lock};
use async_trait::async_trait;
use run_domain::{Run, RunOutcome, RunState};
use run_orchestrator::{OrchestratorError, VmSpecFactory};
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, atomic::Ordering},
    time::Duration,
};
use vm_trait::{RuntimeGitBridge, VmError, VmSpec};
use volume_domain::RunVolumeSelections;

struct GuestFactory {
    cwd: Option<PathBuf>,
    bridge: bool,
    log: OnceLock<Arc<Mutex<Vec<&'static str>>>>,
}
#[async_trait]
impl VmSpecFactory for GuestFactory {
    async fn build(&self, _: &Run) -> Result<VmSpec, VmError> {
        panic!("Invocation must retain the plural factory");
    }
    async fn build_with_volumes(
        &self,
        run: &Run,
        selections: &RunVolumeSelections,
    ) -> Result<VmSpec, VmError> {
        assert_eq!(run.state, RunState::Provisioning);
        lock(self.log.get().unwrap()).push("guest-cwd-factory");
        let mut spec = Factory.build_with_volumes(run, selections).await?;
        spec.command.working_dir.clone_from(&self.cwd);
        spec.runtime_git_bridge = self
            .bridge
            .then(|| RuntimeGitBridge::new(run.id.as_uuid(), 8080));
        Ok(spec)
    }
}

async fn controlled(count: usize, cwd: Option<&str>, bridge: bool) -> Controlled {
    let factory = Arc::new(GuestFactory {
        cwd: cwd.map(PathBuf::from),
        bridge,
        log: OnceLock::new(),
    });
    let controlled = Controlled::with_factory(count, factory.clone()).await;
    factory.log.set(controlled.fixture.log.clone()).unwrap();
    controlled
}

#[tokio::test]
async fn release_guest_cwd_reaches_factory_provision_start_ack_and_attachment_unchanged() {
    for count in [0, 2] {
        for cwd in [None, Some("/release"), Some("/release/app data/子")] {
            let c = controlled(count, cwd, false).await;
            let running = c.start();
            c.ready().await;
            let spec = lock(&c.provider.spec).clone().unwrap();
            assert_eq!(
                spec.command.working_dir.map(PathBuf::into_os_string),
                cwd.map(OsString::from)
            );
            assert!(spec.runtime_git_bridge.is_none());
            assert!(spec.runtime_authority.is_some());
            assert_eq!(spec.disks.len(), count);
            assert_eq!(spec.guest_volumes.len(), count);
            let log = lock(&c.fixture.log).clone();
            let index = |name| log.iter().position(|item| *item == name).unwrap();
            for pair in [
                "current-runtime-prepare",
                "current-empty-secret-prepare",
                "current-authority-prepare",
                "guest-cwd-factory",
                "provision",
                "start",
                "current-authority-ack",
                "current-full-attach",
            ]
            .windows(2)
            {
                assert!(index(pair[0]) < index(pair[1]), "{pair:?}: {log:?}");
            }
            assert_eq!(c.fixture.runs.run.lock().await.state, RunState::Running);
            c.provider.finish(0);
            let run = running.await.unwrap().unwrap();
            assert_eq!(run.state, RunState::CleanedUp);
            assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
            assert!(lock(&c.fixture.volumes.stale).is_empty());
        }
    }
}

async fn refused(c: &Controlled) {
    let error = c.runner.start_run(&c.fixture.command).await.unwrap_err();
    assert!(error.to_string().contains("unsafe guest working directory"));
    assert_eq!(c.fixture.count("guest-cwd-factory"), 1);
    for name in [
        "provision",
        "start",
        "stop",
        "current-authority-ack",
        "current-full-attach",
    ] {
        assert_eq!(c.fixture.count(name), 0, "{name}");
    }
    assert!(lock(&c.provider.spec).is_none());
    // Refusal retains existing owning scoped cleanup, even before provision.
    assert_eq!(c.fixture.count("scoped-cleanup"), 1);
    assert!(c.fixture.cleanup.state.lock().await.receipt.is_some());
    assert!(lock(&c.fixture.volumes.stale).is_empty());
    assert_eq!(c.fixture.runs.run.lock().await.state, RunState::CleanedUp);
}

#[tokio::test]
async fn unsafe_guest_cwd_refuses_before_workload_provider_io_and_cleans_owned_resources() {
    for count in [0, 2] {
        for cwd in [
            "/workspace",
            "/tmp/heph",
            "/",
            "release",
            "/releaseX",
            "/release/../workspace",
            "/release/./bin",
            "/release//bin",
            "/release/bin/",
            "/release/\n",
        ] {
            refused(&controlled(count, Some(cwd), false).await).await;
        }
    }
}

#[tokio::test]
async fn git_bridge_refuses_even_with_absent_or_valid_release_guest_cwd() {
    for count in [0, 2] {
        for cwd in [None, Some("/release")] {
            refused(&controlled(count, cwd, true).await).await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn unsafe_guest_cwd_retains_bounded_uncertain_cleanup_and_all_fences() {
    let c = controlled(2, Some("/workspace"), false).await;
    c.fixture
        .provider
        .hang_cleanup
        .store(true, Ordering::SeqCst);
    let before = tokio::time::Instant::now();
    let error = c.runner.start_run(&c.fixture.command).await.unwrap_err();
    assert!(matches!(error, OrchestratorError::CleanupIncomplete { .. }));
    assert!(error.to_string().contains("unsafe guest working directory"));
    assert!(error.to_string().contains("deadline elapsed"));
    assert_eq!(before.elapsed(), Duration::from_secs(30));
    assert_eq!(c.fixture.count("provision"), 0);
    assert_eq!(c.fixture.count("start"), 0);
    assert_eq!(c.fixture.count("scoped-cleanup"), 1);
    assert_eq!(c.fixture.count("finish-all"), 0);
    assert!(c.fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(lock(&c.fixture.volumes.stale).len(), 2);
}
