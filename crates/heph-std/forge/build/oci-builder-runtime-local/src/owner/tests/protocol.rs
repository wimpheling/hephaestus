use super::*;
use std::os::unix::fs::PermissionsExt;
#[test]
fn caller_abort_before_effect_keeps_registered_original_and_closed_intake() {
    // Existing rt/time/sync dependencies: no tokio test-macro feature needed.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::new();
            let gate = Arc::new(Notify::new());
            let node = fixture.node.clone();
            jobs::start_work(&fixture.node, fixture.owner.clone(), {
                let gate = gate.clone();
                async move {
                    gate.notified().await;
                    assert!(node.open().is_err());
                    failed()
                }
            });
            assert!(lock(&fixture.node.work).is_some() && lock(&fixture.node.watcher).is_some());
            let waiter = tokio::spawn({
                let node = fixture.node.clone();
                async move { node.outcome().await }
            });
            waiter.abort();
            assert!(bounded(waiter).await.err().unwrap().is_cancelled());
            fixture.owner.close();
            fixture.node.kick(fixture.owner.clone());
            assert!(
                observe(
                    &fixture.owner,
                    &[fixture.node.clone()],
                    Duration::from_millis(20)
                )
                .await
                .is_err()
            );
            assert_eq!(fixture.token.attempts.load(Ordering::SeqCst), 0);
            gate.notify_one();
            bounded(observe(
                &fixture.owner,
                &[fixture.node.clone()],
                Duration::from_secs(1),
            ))
            .await
            .unwrap();
        });
}
#[test]
fn stalled_original_provision_cleanup_and_retry_unblock_exact_waiter_without_replacement() {
    // Existing rt/time/sync dependencies: no tokio test-macro feature needed.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::new();
            fixture.scratch();
            fixture.token.paused.store(true, Ordering::SeqCst);
            fixture.token.fail.store(1, Ordering::SeqCst);
            let spec = fixture.spec();
            let node = fixture.node.clone();
            let token = fixture.token.clone();
            jobs::start_work(&fixture.node, fixture.owner.clone(), async move {
                node.install(Phase::Builder, spec, token.clone()).unwrap();
                let result = token.provision().await;
                assert!(result.is_err());
                failed()
            });
            bounded(fixture.token.entered.notified()).await;
            let waiter = tokio::spawn({
                let node = fixture.node.clone();
                async move {
                    let _observed = node.outcome().await;
                    ready(&node).await;
                }
            });
            fixture.node.close();
            fixture.node.kick(fixture.owner.clone());
            assert!(
                observe(
                    &fixture.owner,
                    &[fixture.node.clone()],
                    Duration::from_millis(30)
                )
                .await
                .is_err()
            );
            assert_eq!(fixture.token.cleanups.load(Ordering::SeqCst), 1);
            assert!(fixture.node.paths[0].exists());
            assert!(!waiter.is_finished());
            fixture.node.join_cleanup().await;
            fixture.node.kick(fixture.owner.clone());
            bounded(observe(
                &fixture.owner,
                &[fixture.node.clone()],
                Duration::from_secs(1),
            ))
            .await
            .unwrap();
            bounded(waiter).await.unwrap();
            assert_eq!(fixture.token.attempts.load(Ordering::SeqCst), 1);
            assert!(fixture.token.instance.destroyed.load(Ordering::SeqCst));
            assert!(!fixture.node.paths[0].exists());
        });
}
#[test]
fn start_failure_and_actual_producer_panic_keep_raw_errors_until_same_token_cleanup() {
    // Existing rt/time/sync dependencies: no tokio test-macro feature needed.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::new();
            fixture.scratch();
            let spec = fixture.spec();
            let node = fixture.node.clone();
            let token = fixture.token.clone();
            jobs::start_work(&fixture.node, fixture.owner.clone(), async move {
                node.install(Phase::Builder, spec, token.clone()).unwrap();
                let vm = token.provision().await.unwrap();
                node.instance(Phase::Builder, vm.clone()).unwrap();
                node.error(Arc::new(vm.start().await.unwrap_err()));
                panic!("controlled original producer panic");
            });
            bounded(observe(
                &fixture.owner,
                &[fixture.node.clone()],
                Duration::from_secs(1),
            ))
            .await
            .unwrap();
            assert!(fixture.token.instance.destroyed.load(Ordering::SeqCst));
            assert!(
                lock(&fixture.node.state)
                    .errors
                    .iter()
                    .any(|error| error.downcast_ref::<tokio::task::JoinError>().is_some())
            );
            assert!(
                lock(&fixture.node.state)
                    .errors
                    .iter()
                    .any(|error| error.downcast_ref::<vm_trait::VmError>().is_some())
            );
        });
}
#[test]
fn full_request_conflict_never_replaces_original_or_unrelated_job() {
    // Existing rt/time/sync dependencies: no tokio test-macro feature needed.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::new();
            let mut foreign = fixture.node.request.clone();
            foreign.project_id = uuid::Uuid::new_v4();
            let (same, new, matched) = fixture
                .owner
                .registry
                .admit(foreign, fixture.node.paths.clone())
                .unwrap();
            assert!(Arc::ptr_eq(&same, &fixture.node));
            assert!(!new && !matched);
            let mut other = fixture.node.request.clone();
            other.job_id = uuid::Uuid::new_v4();
            let paths = fixture
                .node
                .paths
                .clone()
                .map(|p| p.with_extension("unrelated"));
            let (other, new, matched) = fixture.owner.registry.admit(other, paths).unwrap();
            assert!(new && matched);
            assert!(!Arc::ptr_eq(&other, &same));
            fixture.node.close();
            assert!(other.open().is_ok());
            assert_eq!(fixture.token.attempts.load(Ordering::SeqCst), 0);
        });
}
#[test]
fn pinned_scratch_replacement_and_outputs_are_preserved_on_cleanup_denial() {
    // Existing rt/time/sync dependencies: no tokio test-macro feature needed.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::new();
            fixture.scratch();
            let original = fixture.temporary.path().join("original-held");
            std::fs::rename(&fixture.node.paths[0], &original).unwrap();
            std::fs::write(&fixture.node.paths[0], b"unrelated replacement").unwrap();

            for (index, parent) in [
                (1, fixture.owner.roots.candidate.clone()),
                (2, fixture.owner.roots.verification.clone()),
            ] {
                fixture.node.acquire(index).unwrap();
                std::fs::create_dir(&fixture.node.paths[index]).unwrap();
                std::fs::set_permissions(
                    &fixture.node.paths[index],
                    std::fs::Permissions::from_mode(0o700),
                )
                .unwrap();
                let file = fixture.node.capture_fd(
                    index,
                    super::super::pin::Pin::directory_file(&fixture.node.paths[index]).unwrap(),
                );
                let pin = super::super::pin::Pin::directory_acquired(
                    file,
                    &fixture.node.paths[index],
                    parent,
                )
                .unwrap();
                fixture.node.leaf(index, pin);
                std::fs::write(
                    fixture.node.paths[index].join("retained-output"),
                    b"original output",
                )
                .unwrap();
            }
            jobs::start_work(&fixture.node, fixture.owner.clone(), async { failed() });
            assert!(
                observe(
                    &fixture.owner,
                    &[fixture.node.clone()],
                    Duration::from_millis(40)
                )
                .await
                .is_err()
            );
            assert!(original.exists());
            for index in [1, 2] {
                assert_eq!(
                    std::fs::read(fixture.node.paths[index].join("retained-output")).unwrap(),
                    b"original output"
                );
                fixture.node.pin(index).unwrap().recheck().unwrap();
            }
            assert_eq!(fixture.node.status().retained_outputs, 2);

            assert_eq!(
                std::fs::read(&fixture.node.paths[0]).unwrap(),
                b"unrelated replacement"
            );
            assert!(matches!(
                lock(&fixture.node.state).leaves[0],
                Leaf::Original(_)
            ));
        });
}
