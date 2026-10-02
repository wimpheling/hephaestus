use super::support::Fixture;
use std::fs;
use volume_trait::VolumeError;

#[tokio::test]
async fn cached_admission_rechecked_under_flock_before_any_birth_journal_write() {
    let fixture = Fixture::new();
    // Admission 1 succeeds before the physical flock; admission 2 simulates a
    // revoked actor or committed permanent birth fence after that cached claim.
    fixture.state.lock().expect("state").reject_begin = Some(2);
    assert!(matches!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await,
        Err(VolumeError::PermissionDenied)
    ));
    assert_eq!(fixture.state.lock().expect("state").begins, 2);
    assert!(
        !fixture
            .root
            .path()
            .join(fixture.request.purpose.namespace_name())
            .exists()
    );
    assert!(!fixture.canonical().exists());
    assert_eq!(fixture.format_count(), 0);
    assert!(
        fixture
            .state
            .lock()
            .expect("state")
            .context
            .observation
            .is_none()
    );
    // Only the prospective owner marker and per-volume exclusion lock exist;
    // journal creation/allocation/formatting have not begun.
    assert_eq!(
        fs::read_dir(fixture.root.path()).expect("entries").count(),
        2
    );
}
