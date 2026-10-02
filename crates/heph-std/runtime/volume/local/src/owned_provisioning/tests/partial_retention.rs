//! Actual host custody tests; memory metadata fixtures make no SQL/fence claims.
mod locks;
mod safety;
mod support;

use super::support::Fixture;
use std::sync::atomic::Ordering;
use support::{Stage, Worker, prepare, snapshot};
use volume_trait::{OwnedBackingPhase, OwnedPartialBirthPhase};

#[tokio::test]
async fn positive_partial_phases_are_readonly_after_original_actor_loss() {
    for (stage, phase, length) in [
        (Stage::Empty, OwnedPartialBirthPhase::ClaimedEmpty, 0),
        (
            Stage::AllocationGap,
            OwnedPartialBirthPhase::AllocatedNeverFormat,
            16 * 1024 * 1024,
        ),
        (
            Stage::Allocated,
            OwnedPartialBirthPhase::AllocatedNeverFormat,
            16 * 1024 * 1024,
        ),
        (
            Stage::FormatIntent,
            OwnedPartialBirthPhase::FormatIntentIncomplete,
            16 * 1024 * 1024,
        ),
    ] {
        let fixture = Fixture::new();
        prepare(&fixture, stage);
        fixture.state.lock().unwrap().authorized = false;
        let worker = Worker::new(&fixture);
        let before = snapshot(fixture.root.path());
        let mut guard = fixture
            .store
            .observe_owned_partial(&worker, &fixture.claim())
            .await
            .unwrap();
        assert_eq!(guard.observation().fields().phase, phase);
        assert_eq!(guard.observation().fields().actual_length_bytes, length);
        assert_eq!(
            guard.observation().fields().declared_capacity_bytes,
            16 * 1024 * 1024
        );
        assert_eq!(guard.observation().fields().observation_version, 1);
        assert!(
            guard
                .observation()
                .matches_creation(fixture.claim().purpose.receipt().intent.creation())
        );
        guard.revalidate().unwrap();
        assert_eq!(before, snapshot(fixture.root.path()));
        assert_eq!(fixture.state.lock().unwrap().begins, 0);
        assert_eq!(worker.records.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.format_count(), 0);
    }
}
#[tokio::test]
async fn clean_ready_commit_gap_and_known_ready_are_not_partial_retirement() {
    let fixture = Fixture::new();
    fixture.state.lock().unwrap().fail_ready_once = true;
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    assert_eq!(
        fixture
            .state
            .lock()
            .unwrap()
            .context
            .observation
            .as_ref()
            .unwrap()
            .fields()
            .phase,
        OwnedBackingPhase::FormatIntent
    );
    let worker = Worker::new(&fixture);
    let before = snapshot(fixture.root.path());
    assert!(
        fixture
            .store
            .observe_owned_partial(&worker, &fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(before, snapshot(fixture.root.path()));
    fixture.state.lock().unwrap().authorized = false;
    fixture
        .store
        .reconcile_owned(&fixture.claim())
        .await
        .expect("ordinary readonly Ready completion");
    assert!(
        fixture
            .store
            .observe_owned_partial(&worker, &fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(fixture.format_count(), 1);
}
#[tokio::test]
async fn formatted_but_incomplete_initial_birth_has_positive_partial_custody() {
    use std::os::unix::fs::FileExt;
    let fixture = Fixture::new();
    fixture.state.lock().unwrap().fail_ready_once = true;
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(fixture.canonical())
        .unwrap();
    file.write_all_at(&[0, 0], 1082).unwrap(); // initial ext4 clean-state proof is unavailable
    file.sync_all().unwrap();
    drop(file);
    let worker = Worker::new(&fixture);
    let before = snapshot(fixture.root.path());
    let mut guard = fixture
        .store
        .observe_owned_partial(&worker, &fixture.claim())
        .await
        .unwrap();
    assert_eq!(
        guard.observation().fields().phase,
        OwnedPartialBirthPhase::FormatIntentIncomplete
    );
    guard.revalidate().unwrap();
    assert_eq!(before, snapshot(fixture.root.path()));
    assert_eq!(fixture.format_count(), 1);
}
