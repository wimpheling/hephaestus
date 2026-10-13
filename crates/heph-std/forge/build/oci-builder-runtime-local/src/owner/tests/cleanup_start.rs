//! Controlled independent original cleanup scheduling, not native VM proof.
use super::super::{mkfs::Original, pin::Pin};
use super::*;
use async_trait::async_trait;
use std::{
    fs::{File, Permissions},
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    process::Command,
};
use vm_trait::{VmError, VmInstance};

struct Cleanup {
    token: Arc<Token>,
    entered: Notify,
    release: Notify,
    hold: AtomicBool,
    fail: AtomicBool,
    calls: AtomicUsize,
    active: AtomicUsize,
}
impl Cleanup {
    fn new(hold: bool, fail: bool) -> Arc<Self> {
        Arc::new(Self {
            token: Token::new(),
            entered: Notify::new(),
            release: Notify::new(),
            hold: AtomicBool::new(hold),
            fail: AtomicBool::new(fail),
            calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
        })
    }
}
#[async_trait]
impl VmProvisionAttempt for Cleanup {
    async fn provision(&self) -> Result<Arc<dyn VmInstance>, VmError> {
        self.token.provision().await
    }
    async fn cleanup(&self) -> Result<(), VmError> {
        assert_eq!(self.active.fetch_add(1, Ordering::SeqCst), 0);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.hold.swap(false, Ordering::SeqCst) {
            self.release.notified().await;
        }
        let result = if self.fail.swap(false, Ordering::SeqCst) {
            Err(VmError::InvalidState("original independent cleanup denied"))
        } else {
            self.token.cleanup().await
        };
        assert_eq!(self.active.fetch_sub(1, Ordering::SeqCst), 1);
        result
    }
}
fn install(fixture: &Fixture, phase: Phase, token: &Arc<Cleanup>) {
    let mut spec = fixture.spec();
    spec.id = token.token.instance.id().clone();
    fixture.node.install(phase, spec, token.clone()).unwrap();
}
fn formatter(fixture: &Fixture) {
    let mut command = Command::new(fixture.temporary.path().join("missing-original-mkfs"));
    let original = Original::new(&command);
    fixture.node.mkfs(original.clone()).unwrap();
    // Actual known spawn denial: no formatter/native fixture is needed here.
    assert!(original.run(&mut command).is_err());
}
struct FormatterGuard {
    fixture: Option<Fixture>,
    original: Option<Arc<Original>>,
}
impl Drop for FormatterGuard {
    fn drop(&mut self) {
        if let Some(original) = &self.original {
            let stopped = original.bounded_stop(Duration::from_secs(1));
            if !stopped || original.helper_unqualified() {
                if let Some(fixture) = self.fixture.take() {
                    std::mem::forget(fixture);
                }
                eprintln!("original formatter helper uncertainty: fixture preserved");
            }
        }
    }
}
fn running_formatter(guard: &mut FormatterGuard) {
    let node = guard.fixture.as_ref().unwrap().node.clone();
    let mut command = Command::new("/bin/sleep");
    command.arg("30");
    let original = Original::new(&command);
    node.mkfs(original.clone()).unwrap();
    guard.original = Some(original.clone());
    let owner = node.clone();
    let (release, registered) = tokio::sync::oneshot::channel();
    let task = super::super::task::Task::new(tokio::task::spawn_blocking(move || {
        registered.blocking_recv().unwrap();
        if let Err(error) = original.run(&mut command) {
            owner.error(Arc::new(error));
        }
        // Actual original parent only, not a formatter/guest/helper certificate.
        Err::<Arc<crate::constants::ScratchDisk>, _>(Arc::new(
            oci_builder_worker::OciWorkerError::InvalidConfiguration,
        ))
    }));
    lock(&node.state).preparation = Some(task);
    let _registered = release.send(());
}
async fn formatter_joined(fixture: &Fixture) {
    bounded(async {
        loop {
            let task = { lock(&fixture.node.mkfs_cleanup).last().cloned() };
            if let Some(task) = task {
                assert!(task.join(&fixture.node).await.unwrap().is_ok());
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}
fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}

#[test]
fn stalled_builder_starts_verifier_and_formatter_cancellation_preserves_cleanup_handles() {
    run(async {
        let mut guard = FormatterGuard {
            fixture: Some(Fixture::new()),
            original: None,
        };
        let fixture = guard.fixture.as_ref().unwrap();
        fixture.scratch();
        let builder = Cleanup::new(true, false);
        let verifier = Cleanup::new(false, false);
        install(fixture, Phase::Builder, &builder);
        install(fixture, Phase::Verifier, &verifier);
        running_formatter(&mut guard);
        let fixture = guard.fixture.as_ref().unwrap();
        jobs::start_work(&fixture.node, fixture.owner.clone(), async { failed() });
        bounded(builder.entered.notified()).await;
        bounded(verifier.entered.notified()).await;
        formatter_joined(fixture).await;
        assert_eq!(
            guard.original.as_ref().unwrap().status().unwrap().signal(),
            Some(9)
        );
        assert_eq!(builder.active.load(Ordering::SeqCst), 1);
        assert!(verifier.token.instance.destroyed.load(Ordering::SeqCst));
        let original = lock(&fixture.node.cleanup).as_ref().unwrap().clone();
        let observer = tokio::spawn({
            let node = fixture.node.clone();
            async move { node.detach().await }
        });
        tokio::task::yield_now().await;
        observer.abort();
        assert!(observer.await.unwrap_err().is_cancelled());
        assert!(
            tokio::time::timeout(Duration::from_millis(1), fixture.node.detach())
                .await
                .is_err()
        );
        fixture.node.kick(fixture.owner.clone());
        assert!(Arc::ptr_eq(
            &original,
            lock(&fixture.node.cleanup).as_ref().unwrap()
        ));
        assert_eq!(builder.calls.load(Ordering::SeqCst), 1);
        assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
        assert!(!fixture.node.delivery_joined());
        builder.release.notify_one();
        ready(&fixture.node).await;
        assert!(fixture.node.require_detached().is_ok());
        assert_eq!(
            fixture.node.status().phase,
            OciCleanupPhase::ScratchHelperUnqualified
        );
        assert_eq!(builder.calls.load(Ordering::SeqCst), 1);
        assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn first_token_failure_still_starts_every_original_then_exact_retry_settles() {
    run(async {
        let fixture = Fixture::new();
        let builder = Cleanup::new(false, true);
        let verifier = Cleanup::new(false, false);
        install(&fixture, Phase::Builder, &builder);
        install(&fixture, Phase::Verifier, &verifier);
        formatter(&fixture);
        jobs::start_work(&fixture.node, fixture.owner.clone(), async { failed() });
        bounded(builder.entered.notified()).await;
        bounded(verifier.entered.notified()).await;
        formatter_joined(&fixture).await;
        assert!(
            observe(
                &fixture.owner,
                &[fixture.node.clone()],
                Duration::from_millis(20)
            )
            .await
            .is_err()
        );
        assert_eq!(builder.calls.load(Ordering::SeqCst), 1);
        assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
        assert!(verifier.token.instance.destroyed.load(Ordering::SeqCst));
        assert!(!fixture.node.delivery_joined());
        fixture.node.join_cleanup().await;
        fixture.node.kick(fixture.owner.clone());
        ready(&fixture.node).await;
        assert_eq!(fixture.node.status().phase, OciCleanupPhase::Settled);
        assert_eq!(builder.calls.load(Ordering::SeqCst), 2);
        assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
        let state = lock(&fixture.node.state);
        assert_eq!(state.births[&Phase::Builder].cleanup_history.len(), 2);
        assert_eq!(state.births[&Phase::Verifier].cleanup_history.len(), 1);
        assert!(state.errors.iter().any(|error| {
            error
                .to_string()
                .contains("original independent cleanup denied")
        }));
        drop(state);
    });
}

#[test]
fn validation_failure_keeps_exact_acquired_fd_in_original_slot() {
    let fixture = Fixture::new();
    fixture.node.acquire(0).unwrap();
    let file = File::create(&fixture.node.paths[0]).unwrap();
    let file = fixture.node.capture_fd(0, file);
    file.set_permissions(Permissions::from_mode(0o666)).unwrap();
    assert!(
        Pin::scratch(
            file.clone(),
            &fixture.node.paths[0],
            fixture.owner.roots.scratch.clone()
        )
        .is_err()
    );
    let state = lock(&fixture.node.state);
    let Leaf::Acquiring {
        _original_file: Some(original),
        ..
    } = &state.leaves[0]
    else {
        panic!("actual acquisition must retain original FD before validation");
    };
    assert!(Arc::ptr_eq(original, &file));
    assert_eq!(
        original.metadata().unwrap().permissions().mode() & 0o777,
        0o666
    );
    drop(state);
}
