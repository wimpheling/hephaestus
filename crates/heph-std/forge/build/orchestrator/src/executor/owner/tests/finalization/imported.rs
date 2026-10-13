//! Controlled completion context/actual joins and local sentinels, not PG receipts.
use super::super::super::node::Node;
use super::*;
use crate::{BuildExecutionError, executor::owner::Registry};
use release_domain::{ArtifactKind, ArtifactPath, ContentHash, ReleaseArtifactId};
use release_service::ReleaseArtifactInput;

fn artifacts() -> Vec<ReleaseArtifactInput> {
    vec![ReleaseArtifactInput {
        id: ReleaseArtifactId::new(),
        path: ArtifactPath::parse("bin/agent").unwrap(),
        kind: ArtifactKind::Executable,
        mode: 0o555,
        content_hash: ContentHash::digest(b"old"),
        size_bytes: 3,
        media_type: "application/octet-stream".into(),
        storage_key: uuid::Uuid::new_v4(),
    }]
}
fn copy_claim(original: &ClaimedBuild) -> ClaimedBuild {
    ClaimedBuild {
        input: original.input.clone(),
        release_id: original.release_id,
        release_agent_id: original.release_agent_id,
        release_version: original.release_version.clone(),
    }
}
fn result(original: &ClaimedBuild) -> super::super::super::super::BuildExecutionResult {
    super::super::super::super::BuildExecutionResult {
        build_request_id: original.input.id,
        release_id: original.release_id,
        release_agent_id: original.release_agent_id,
        release_version: original.release_version.clone(),
        artifact_count: 1,
    }
}

#[test]
fn cold_imported_context_never_adopts_or_deletes_even_an_existing_named_directory() {
    use std::os::unix::fs::MetadataExt;
    let temporary = tempfile::tempdir().unwrap();
    let node = Node::new(BuildRequestId::new(), Mode::Execute);
    let unrelated = temporary.path().join(node.id.to_string());
    std::fs::create_dir(&unrelated).unwrap();
    std::fs::write(unrelated.join("sentinel"), b"unrelated original").unwrap();
    let identity = std::fs::metadata(&unrelated).unwrap();
    let original = claimed(node.id);
    let manifest = artifacts();
    assert!(node.prepare_completion(&original, &manifest).is_err());
    node.prepare_imported_completion(&original, &manifest)
        .unwrap();
    node.prepare_completion(&original, &manifest).unwrap();
    assert!(node.original_workspace().is_err());
    assert!(node.require_original_finalization().is_err()); // Never a sealed branch.
    node.completed_result(&result(&original)).unwrap();
    node.finalized();
    node.remove_workspace().unwrap();
    let after = std::fs::metadata(&unrelated).unwrap();
    assert_eq!((after.dev(), after.ino()), (identity.dev(), identity.ino()));
    assert_eq!(
        std::fs::read(unrelated.join("sentinel")).unwrap(),
        b"unrelated original"
    );
    let state = lock(&node.state);
    assert!(state.attempt.is_none() && state.instance.is_none());
    assert!(state.preparation.is_none() && state.git_children.is_empty());
    drop(state);
}

#[test]
fn every_original_claim_field_is_retained_and_changed_retry_cannot_rebind() {
    let node = Node::new(BuildRequestId::new(), Mode::Execute);
    let original = claimed(node.id);
    let manifest = artifacts();
    node.prepare_imported_completion(&original, &manifest)
        .unwrap();
    let retained = lock(&node.state).completion.as_ref().unwrap().clone();
    for field in 0..9 {
        let mut changed = copy_claim(&original);
        match field {
            0 => changed.input.id = BuildRequestId::new(),
            1 => changed.input.repository_id = uuid::Uuid::new_v4(),
            2 => changed.input.source_commit = "b".repeat(40),
            3 => changed.input.source_ref = "refs/heads/foreign".into(),
            4 => changed.input.build.arguments.push("changed".into()),
            5 => changed.input.image_reference = "foreign@sha256:changed".into(),
            6 => changed.release_id = ReleaseId::new(),
            7 => changed.release_agent_id = ReleaseAgentId::new(),
            8 => changed.release_version = ReleaseVersion::parse("changed").unwrap(),
            _ => unreachable!(),
        }
        assert!(
            node.prepare_imported_completion(&changed, &manifest)
                .is_err()
        );
        assert!(Arc::ptr_eq(
            &retained,
            lock(&node.state).completion.as_ref().unwrap()
        ));
    }
    node.prepare_completion(&original, &manifest).unwrap();
}

#[test]
fn every_artifact_field_order_and_cardinality_are_retained_before_publication() {
    let node = Node::new(BuildRequestId::new(), Mode::Execute);
    let original = claimed(node.id);
    let mut manifest = artifacts();
    let mut second = manifest[0].clone();
    second.id = ReleaseArtifactId::new();
    second.path = ArtifactPath::parse("bin/second").unwrap();
    manifest.push(second);
    node.prepare_imported_completion(&original, &manifest)
        .unwrap();
    for field in 0..10 {
        let mut changed = manifest.clone();
        match field {
            0 => changed[0].id = ReleaseArtifactId::new(),
            1 => changed[0].path = ArtifactPath::parse("bin/foreign").unwrap(),
            2 => changed[0].kind = ArtifactKind::File,
            3 => changed[0].mode = 0o444,
            4 => changed[0].content_hash = ContentHash::digest(b"different"),
            5 => changed[0].size_bytes += 1,
            6 => changed[0].media_type = "text/plain".into(),
            7 => changed[0].storage_key = uuid::Uuid::new_v4(),
            8 => changed.reverse(),
            9 => {
                changed.pop();
            }
            _ => unreachable!(),
        }
        assert!(node.prepare_completion(&original, &changed).is_err());
    }
    node.prepare_completion(&original, &manifest).unwrap();
}

#[test]
fn acquiring_and_actual_installed_vm_custody_cannot_become_cold_imported() {
    let node = Node::new(BuildRequestId::new(), Mode::Execute);
    let original = claimed(node.id);
    let manifest = artifacts();
    node.begin_workspace(std::path::PathBuf::from("unmaterialized"))
        .unwrap();
    assert!(
        node.prepare_imported_completion(&original, &manifest)
            .is_err()
    );
    assert!(lock(&node.state).completion.is_none());
    let held = Node::new(BuildRequestId::new(), Mode::Execute);
    let held_claim = claimed(held.id);
    let token = Token::new();
    held.install_attempt(token.clone()).unwrap();
    assert!(
        held.prepare_imported_completion(&held_claim, &manifest)
            .is_err()
    );
    assert!(lock(&held.state).completion.is_none());
    assert_eq!(token.provisions.load(Ordering::SeqCst), 0);
    assert_eq!(token.cleanups.load(Ordering::SeqCst), 0);
}

#[test]
fn warm_context_keeps_original_pin_and_replacement_cannot_use_cold_fallback() {
    let fixture = Fixture::new(Mode::Execute);
    let original = claimed(fixture.node.id);
    let manifest = artifacts();
    fixture
        .node
        .prepare_imported_completion(&original, &manifest)
        .unwrap();
    assert!(Arc::ptr_eq(
        &fixture.node.original_workspace().unwrap(),
        &fixture.pin
    ));
    fixture.node.require_original_finalization().unwrap();
    let relocated = fixture.temporary.path().join("relocated");
    std::fs::rename(fixture.path(), &relocated).unwrap();
    std::fs::create_dir(fixture.path()).unwrap();
    assert!(
        fixture
            .node
            .prepare_completion(&original, &manifest)
            .is_err()
    );
    assert!(fixture.node.original_workspace().is_ok());
    assert_eq!(
        std::fs::read(relocated.join("output")).unwrap(),
        b"original sealed output"
    );
}

#[tokio::test]
async fn lost_completion_reply_keeps_same_context_actual_jobs_and_validates_cached_receipt() {
    let registry = Registry::default();
    let Admission::Start(node, _) = registry
        .register(BuildRequestId::new(), Mode::Execute)
        .unwrap()
    else {
        panic!("original imported owner");
    };
    let original = Arc::new(claimed(node.id));
    let manifest = Arc::new(artifacts());
    jobs::start(&node, {
        let node = node.clone();
        let original = original.clone();
        let manifest = manifest.clone();
        async move {
            node.prepare_imported_completion(&original, &manifest)
                .unwrap();
            node.preserve_finalization();
            // Controlled lost publication reply; no actual database receipt.
            Outcome::Build(Err(Arc::new(BuildExecutionError::Release)))
        }
    });
    node.outcome().await;
    watcher_finished(&node).await;
    let retained = lock(&node.state).completion.as_ref().unwrap().clone();
    registry.close();
    let Admission::Start(continued, Mode::Finalize) =
        registry.register(node.id, Mode::Finalize).unwrap()
    else {
        panic!("same saved context after actual joins");
    };
    assert!(Arc::ptr_eq(&continued, &node));
    jobs::start(&continued, {
        let owner = continued.clone();
        let original = original.clone();
        let manifest = manifest.clone();
        async move {
            owner.prepare_completion(&original, &manifest).unwrap();
            let mut receipt = result(&original);
            receipt.artifact_count += 1;
            assert!(owner.completed_result(&receipt).is_err());
            receipt.artifact_count = 1;
            owner.completed_result(&receipt).unwrap();
            owner.finalized();
            owner.remove_workspace().unwrap();
            Outcome::Build(Ok(receipt))
        }
    });
    continued.outcome().await;
    bounded(observe(&[continued.clone()], Duration::from_secs(1)))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(
        &retained,
        lock(&continued.state).completion.as_ref().unwrap()
    ));
    assert_eq!(lock(&continued.state).history.len(), 1);
}

#[test]
fn cached_completed_receipt_must_match_every_available_result_field() {
    let node = Node::new(BuildRequestId::new(), Mode::Execute);
    let original = claimed(node.id);
    node.prepare_imported_completion(&original, &artifacts())
        .unwrap();
    for field in 0..5 {
        let mut changed = result(&original);
        match field {
            0 => changed.build_request_id = BuildRequestId::new(),
            1 => changed.release_id = ReleaseId::new(),
            2 => changed.release_agent_id = ReleaseAgentId::new(),
            3 => changed.release_version = ReleaseVersion::parse("foreign").unwrap(),
            4 => changed.artifact_count += 1,
            _ => unreachable!(),
        }
        assert!(node.completed_result(&changed).is_err());
    }
    node.completed_result(&result(&original)).unwrap();
}

#[test]
fn unfinalized_hold_and_unknown_cleanup_errors_retain_original_error_context() {
    for failure in [
        BuildExecutionError::Filesystem,
        BuildExecutionError::VmCleanup,
    ] {
        let node = Node::new(BuildRequestId::new(), Mode::Execute);
        let original = claimed(node.id);
        let manifest = artifacts();
        node.prepare_imported_completion(&original, &manifest)
            .unwrap();
        let context = lock(&node.state).completion.as_ref().unwrap().clone();
        let hold = Arc::new(BuildExecutionError::FinalizationRequired);
        lock(&node.state).cleanup_error = Some(hold.clone());
        node.preserve_finalization();
        assert!(node.remove_workspace().is_err());
        assert!(Arc::ptr_eq(
            &hold,
            lock(&node.state).cleanup_error.as_ref().unwrap()
        ));
        node.finalized();
        let unknown = Arc::new(failure);
        lock(&node.state).cleanup_error = Some(unknown.clone());
        node.error(unknown.clone());
        node.remove_workspace().unwrap();
        let state = lock(&node.state);
        assert!(Arc::ptr_eq(&unknown, state.cleanup_error.as_ref().unwrap()));
        assert!(Arc::ptr_eq(&context, state.completion.as_ref().unwrap()));
        assert_eq!(state.errors.len(), 1);
        drop(state);
        assert!(node.prepare_completion(&original, &manifest).is_err());
    }
}
