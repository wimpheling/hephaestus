use super::*;

#[tokio::test]
async fn abandoned_provision_waiter_and_execution_panic_preserve_exact_token() {
    let fixture = Fixture::new(Mode::Execute);
    fixture.token.provision_paused.store(true, Ordering::SeqCst);
    let node = fixture.node.clone();
    let token = fixture.token.clone();
    jobs::start(&fixture.node, async move {
        node.install_attempt(token.clone()).unwrap();
        let error = token.provision().await.err().unwrap();
        node.vm_error(error);
        failure()
    });
    bounded(fixture.token.provision_entered.notified()).await;
    let waiter = tokio::spawn({
        let node = fixture.node.clone();
        async move { node.outcome().await }
    });
    waiter.abort();
    assert!(bounded(waiter).await.err().unwrap().is_cancelled());
    fixture.registry.close();
    fixture.node.kick_cleanup(true);
    bounded(observe(&[fixture.node.clone()], Duration::from_secs(1)))
        .await
        .unwrap();
    assert_eq!(fixture.token.provisions.load(Ordering::SeqCst), 1);
    assert!(fixture.token.instance.destroyed.load(Ordering::SeqCst));
    let panic_fixture = Fixture::new(Mode::Execute);
    let node = panic_fixture.node.clone();
    let token = panic_fixture.token.clone();
    jobs::start(&panic_fixture.node, async move {
        node.install_attempt(token.clone()).unwrap();
        let instance = token.provision().await.unwrap();
        node.instance(instance);
        panic!("controlled execution panic after original provision")
    });
    bounded(observe(
        &[panic_fixture.node.clone()],
        Duration::from_secs(1),
    ))
    .await
    .unwrap();
    assert!(
        lock(&panic_fixture.node.state)
            .errors
            .iter()
            .any(|error| error.downcast_ref::<tokio::task::JoinError>().is_some())
    );
    assert!(
        panic_fixture
            .token
            .instance
            .destroyed
            .load(Ordering::SeqCst)
    );
}
#[tokio::test]
async fn cleanup_budget_never_aborts_original_and_concurrent_retry_cannot_duplicate_pass() {
    let fixture = Fixture::new(Mode::Verify);
    fixture.token.instance.paused.store(true, Ordering::SeqCst);
    let node = fixture.node.clone();
    let token = fixture.token.clone();
    jobs::start(&fixture.node, async move {
        node.install_attempt(token.clone()).unwrap();
        let instance = token.provision().await.unwrap();
        node.instance(instance);
        Outcome::Verify(Ok(()))
    });
    fixture.node.outcome().await;
    fixture.registry.close();
    fixture.node.kick_cleanup(true);
    bounded(fixture.token.instance.entered.notified()).await;
    assert!(
        observe(&[fixture.node.clone()], Duration::from_millis(20))
            .await
            .is_err()
    );
    fixture.node.kick_cleanup(true); // SAME actual in-flight cleanup, no overlap.
    assert_eq!(fixture.token.cleanups.load(Ordering::SeqCst), 1);
    assert!(!fixture.token.instance.destroyed.load(Ordering::SeqCst));
    assert!(fixture.path().exists());
    fixture.token.instance.released.notify_one();
    bounded(observe(&[fixture.node.clone()], Duration::from_secs(1)))
        .await
        .unwrap();
    assert_eq!(fixture.token.cleanups.load(Ordering::SeqCst), 1);
    assert!(!fixture.path().exists());
}
#[tokio::test]
async fn replaced_workspace_refuses_cleanup_and_preserves_both_original_and_replacement() {
    let fixture = Fixture::new(Mode::Verify);
    jobs::start(&fixture.node, async { Outcome::Verify(Ok(())) });
    fixture.node.outcome().await;
    watcher_finished(&fixture.node).await;
    let original = fixture.temporary.path().join("held-original");
    std::fs::rename(fixture.path(), &original).unwrap();
    std::fs::create_dir(fixture.path()).unwrap();
    std::fs::write(fixture.path().join("sentinel"), b"unrelated replacement").unwrap();
    fixture.node.close();
    fixture.node.kick_cleanup(true);
    assert!(
        observe(&[fixture.node.clone()], Duration::from_millis(40))
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(original.join("output")).unwrap(),
        b"original sealed output"
    );
    assert_eq!(
        std::fs::read(fixture.path().join("sentinel")).unwrap(),
        b"unrelated replacement"
    );
    // Test-controlled restoration of the SAME pinned inode is not a product
    // reacquisition/adoption mechanism. Retry still uses original descriptors.
    let unrelated = fixture.temporary.path().join("replacement");
    std::fs::rename(fixture.path(), &unrelated).unwrap();
    std::fs::rename(&original, fixture.path()).unwrap();
    fixture.node.kick_cleanup(true);
    bounded(observe(&[fixture.node.clone()], Duration::from_secs(1)))
        .await
        .unwrap();
    assert!(unrelated.join("sentinel").exists());
    assert!(fixture.pin.removed());
}
