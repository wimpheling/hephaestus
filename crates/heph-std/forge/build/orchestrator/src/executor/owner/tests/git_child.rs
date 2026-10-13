//! Actual original subprocess controls; not VM/PG or arbitrary subtree proof.
use super::super::super::{
    BuildExecutorConfig,
    workspace::{git_child::GitChild, git_command},
};
use super::*;
use std::{os::unix::process::ExitStatusExt, path::PathBuf, process::Command};

struct OriginalFixture {
    fixture: Option<Fixture>,
    child: Arc<GitChild>,
}
impl Drop for OriginalFixture {
    fn drop(&mut self) {
        if !self.child.bounded_test_stop(Duration::from_secs(1)) {
            // Explicit uncertainty: preserve original filesystem/registry rather
            // than TempDir deleting it while original child may still be alive.
            if let Some(fixture) = self.fixture.take() {
                std::mem::forget(fixture);
            }
            eprintln!("original test Child death unconfirmed; original fixture retained");
        }
    }
}
#[tokio::test]
async fn abandoned_preparation_waiter_keeps_live_original_child_until_actual_wait_and_io_join() {
    let fixture = Fixture::new(Mode::Verify);
    let node = fixture.node.clone();
    let (sent, receive) = tokio::sync::oneshot::channel();
    let release = Arc::new(Notify::new());
    jobs::start(&fixture.node, {
        let release = release.clone();
        async move {
            let mut command = Command::new("/bin/sleep");
            command.arg("30");
            let child = GitChild::spawn(&mut command, &node).unwrap();
            sent.send(child)
                .unwrap_or_else(|_| panic!("test original owner receiver lost"));
            release.notified().await;
            Outcome::Verify(Ok(()))
        }
    });
    let child = bounded(receive).await.unwrap();
    let guard = OriginalFixture {
        fixture: Some(fixture),
        child: child.clone(),
    };
    let fixture = guard.fixture.as_ref().unwrap();
    assert!(child.observed_exit().is_none());
    let waiter = tokio::spawn({
        let node = fixture.node.clone();
        async move { node.outcome().await }
    });
    waiter.abort();
    assert!(bounded(waiter).await.err().unwrap().is_cancelled());
    assert!(child.observed_exit().is_none());
    assert!(fixture.path().exists());
    fixture.registry.close();
    fixture.node.kick_cleanup(true);
    bounded(async {
        while child.observed_exit().is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(child.observed_exit().unwrap().signal(), Some(9));
    assert!(
        observe(&[fixture.node.clone()], Duration::from_millis(20))
            .await
            .is_err()
    );
    assert!(fixture.path().exists()); // DB/preparation owner still unsettled.
    release.notify_one();
    bounded(observe(&[fixture.node.clone()], Duration::from_secs(1)))
        .await
        .unwrap();
    assert!(!fixture.path().exists());
    drop(child);
    drop(guard);
}
fn config(fixture: &Fixture) -> BuildExecutorConfig {
    BuildExecutorConfig {
        workspace_root: fixture.temporary.path().to_path_buf(),
        repository_root: fixture.temporary.path().to_path_buf(),
        git_binary: std::env::var_os("HEPHAESTUS_TEST_GIT_BINARY")
            .map_or_else(|| PathBuf::from("/usr/bin/git"), PathBuf::from),
        image_filesystems: Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new())),
        timeout: Duration::from_secs(1),
    }
}
fn setup_git(binary: &std::path::Path, args: &[&std::ffi::OsStr]) {
    let output = Command::new(binary)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "controlled setup Git command failed"
    );
}
#[test]
fn builtin_global_option_precedes_repository_and_missing_promisor_object_cannot_fetch() {
    use std::ffi::OsStr;
    let fixture = Fixture::new(Mode::Execute);
    let config = config(&fixture);
    let repository = fixture.temporary.path().join("bare.git");
    setup_git(
        &config.git_binary,
        &[
            OsStr::new("init"),
            OsStr::new("--bare"),
            repository.as_os_str(),
        ],
    );
    let marker = fixture.temporary.path().join("forbidden-fetch");
    for (key, value) in [
        ("extensions.partialClone", "fixture".to_owned()),
        ("remote.fixture.promisor", "true".to_owned()),
        ("protocol.ext.allow", "always".to_owned()),
        (
            "remote.fixture.url",
            format!("ext::/usr/bin/touch {}", marker.display()),
        ),
    ] {
        setup_git(
            &config.git_binary,
            &[
                OsStr::new("--git-dir"),
                repository.as_os_str(),
                OsStr::new("config"),
                OsStr::new(key),
                OsStr::new(&value),
            ],
        );
    }
    let missing = "a".repeat(40);
    let mut command = git_command(&config, &repository, &["cat-file", "blob", &missing]);
    let args = command
        .get_args()
        .map(std::borrow::ToOwned::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(args[0], "--no-lazy-fetch");
    assert_eq!(args[1], "--git-dir");
    assert_eq!(args[2], repository.as_os_str());
    let original = GitChild::spawn(&mut command, &fixture.node).unwrap();
    let output = original.output().unwrap();
    assert!(!output.status.success());
    assert!(!marker.exists());
    original.stop_and_join().unwrap();
    // Positive control of the actual promisor fixture: without the global
    // option Git really invokes this immediate, test-owned touch helper. This
    // is not arbitrary-descendant cleanup proof or production behavior.
    let output = Command::new(&config.git_binary)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("--git-dir")
        .arg(&repository)
        .args(["cat-file", "blob", &missing])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(marker.exists());
}
