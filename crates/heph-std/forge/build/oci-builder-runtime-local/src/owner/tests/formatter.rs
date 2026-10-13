use super::super::mkfs::Original;
use super::*;
use std::{os::unix::process::ExitStatusExt, process::Command};
#[test]
fn known_formatter_spawn_failure_joins_no_child_and_retires_only_pinned_scratch() {
    // Existing rt/time/sync dependencies: no tokio test-macro feature needed.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::new();
            fixture.scratch();
            let node = fixture.node.clone();
            let missing = fixture.temporary.path().join("nonexistent-mkfs");
            jobs::start_work(&fixture.node, fixture.owner.clone(), async move {
                let command = Command::new(missing);
                let original = Original::new(&command);
                node.mkfs(original.clone()).unwrap();
                assert!(original.run(&mut { command }).is_err());
                failed()
            });
            bounded(observe(
                &fixture.owner,
                &[fixture.node.clone()],
                Duration::from_secs(1),
            ))
            .await
            .unwrap();
            assert!(!fixture.node.paths[0].exists());
            assert!(
                !lock(&fixture.node.state)
                    .mkfs
                    .as_ref()
                    .unwrap()
                    .helper_unqualified()
            );
            assert!(!lock(&fixture.node.state).errors.is_empty());
        });
}
struct ParentGuard {
    fixture: Option<Fixture>,
    child: Arc<Original>,
}
impl Drop for ParentGuard {
    fn drop(&mut self) {
        let parent_stopped = self.child.bounded_stop(Duration::from_secs(1));
        if !parent_stopped || self.child.helper_unqualified() {
            // No fixture deletion based on parent-only death when the same
            // production helper projection deliberately remains unqualified.
            if let Some(fixture) = self.fixture.take() {
                std::mem::forget(fixture);
            }
            eprintln!("original formatter helper profile unresolved; original fixture retained");
        }
    }
}
#[test]
fn actual_formatter_parent_stopped_joined_but_unqualified_helpers_keep_scratch_held() {
    // Existing rt/time/sync dependencies: no tokio test-macro feature needed.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::new();
            fixture.scratch();
            let node = fixture.node.clone();
            let (sent, received) = tokio::sync::oneshot::channel();
            jobs::start_work(&fixture.node, fixture.owner.clone(), async move {
                let mut command = Command::new("/bin/sleep");
                command.arg("30");
                let original = Original::new(&command);
                node.mkfs(original.clone()).unwrap();
                let clone = original.clone();
                let (release, registered) = tokio::sync::oneshot::channel();
                let task = super::super::task::Task::new(tokio::task::spawn_blocking(move || {
                    registered.blocking_recv().unwrap();
                    let _status = original.run(&mut command);
                    // This controlled sleeper is NOT a filesystem formatter/receipt.
                    Err::<Arc<crate::constants::ScratchDisk>, _>(Arc::new(
                        oci_builder_worker::OciWorkerError::InvalidConfiguration,
                    ))
                }));
                lock(&node.state).preparation = Some(task.clone());
                sent.send(clone)
                    .unwrap_or_else(|_| panic!("original fixture receiver lost"));
                let _registered = release.send(());
                drop(task.join(&node).await);
                failed()
            });
            let child = bounded(received).await.unwrap();
            let guard = ParentGuard {
                fixture: Some(fixture),
                child: child.clone(),
            };
            let fixture = guard.fixture.as_ref().unwrap();
            bounded(async {
                while child.status().is_none() {
                    fixture.node.close();
                    fixture.node.kick(fixture.owner.clone());
                    fixture.node.join_cleanup().await;
                    tokio::task::yield_now().await;
                }
            })
            .await;
            assert_eq!(child.status().unwrap().signal(), Some(9));
            assert!(
                observe(
                    &fixture.owner,
                    &[fixture.node.clone()],
                    Duration::from_millis(40)
                )
                .await
                .is_err()
            );
            assert!(fixture.node.paths[0].exists());
            assert_eq!(
                fixture.node.status().phase,
                OciCleanupPhase::ScratchHelperUnqualified
            );
            drop(child);
            drop(guard);
        });
}
