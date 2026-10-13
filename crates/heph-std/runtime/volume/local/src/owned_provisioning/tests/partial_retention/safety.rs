use super::{Fixture, Stage, Worker, prepare, snapshot};
use std::{
    fs,
    os::unix::fs::{FileExt, symlink},
};

#[tokio::test]
async fn unknown_birth_and_foreign_matching_geometry_are_never_adopted() {
    let fixture = Fixture::new();
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
    fs::write(fixture.canonical(), b"independently-existing-raw").unwrap();
    let before = snapshot(fixture.root.path());
    assert!(
        fixture
            .store
            .observe_owned_partial(&worker, &fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(before, snapshot(fixture.root.path()));
}
#[tokio::test]
async fn actual_root_replacement_is_denied_without_writes_in_foreign_root() {
    let fixture = Fixture::new();
    prepare(&fixture, Stage::Allocated);
    let worker = Worker::new(&fixture);
    let saved = fixture.root.path().with_extension("partial-owned-saved");
    fs::rename(fixture.root.path(), &saved).unwrap();
    fs::create_dir(fixture.root.path()).unwrap();
    fs::write(fixture.root.path().join("foreign"), b"leave-root-unchanged").unwrap();
    let before = snapshot(fixture.root.path());
    assert!(
        fixture
            .store
            .observe_owned_partial(&worker, &fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(before, snapshot(fixture.root.path()));
    fs::remove_dir_all(fixture.root.path()).unwrap();
    fs::rename(saved, fixture.root.path()).unwrap();
}
#[tokio::test]
async fn custody_revalidation_detects_backing_write_and_inode_substitution() {
    let fixture = Fixture::new();
    prepare(&fixture, Stage::Allocated);
    let worker = Worker::new(&fixture);
    let mut guard = fixture
        .store
        .observe_owned_partial(&worker, &fixture.claim())
        .await
        .unwrap();
    let file = fs::OpenOptions::new()
        .write(true)
        .open(fixture.canonical())
        .unwrap();
    file.write_all_at(b"changed", 32).unwrap();
    file.sync_all().unwrap();
    assert!(guard.revalidate().is_err());
    drop(file);
    drop(guard);
    let mut guard = fixture
        .store
        .observe_owned_partial(&worker, &fixture.claim())
        .await
        .unwrap();
    let saved = fixture.canonical().with_extension("saved");
    fs::rename(fixture.canonical(), &saved).unwrap();
    fs::write(fixture.canonical(), b"substituted").unwrap();
    assert!(guard.revalidate().is_err());
    assert_eq!(fs::read(fixture.canonical()).unwrap(), b"substituted");
}
#[tokio::test]
async fn symlink_backing_and_contradictory_pending_record_remain_held() {
    let fixture = Fixture::new();
    prepare(&fixture, Stage::Allocated);
    let worker = Worker::new(&fixture);
    let target = fixture.canonical().with_extension("foreign");
    fs::write(&target, b"foreign").unwrap();
    fs::remove_file(fixture.canonical()).unwrap();
    symlink(&target, fixture.canonical()).unwrap();
    assert!(
        fixture
            .store
            .observe_owned_partial(&worker, &fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), b"foreign");
    let fixture = Fixture::new();
    prepare(&fixture, Stage::Allocated);
    let worker = Worker::new(&fixture);
    let pending = fixture
        .root
        .path()
        .join(fixture.journal_purpose().namespace_name())
        .join("format-intent.pending");
    fs::write(&pending, b"partial-marker").unwrap();
    let before = snapshot(fixture.root.path());
    assert!(
        fixture
            .store
            .observe_owned_partial(&worker, &fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(before, snapshot(fixture.root.path()));
}

#[tokio::test]
async fn independent_formatted_inode_with_matching_uuid_and_capacity_is_not_adopted() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    prepare(&fixture, Stage::FormatIntent);
    let foreign = fixture.tools.path().join("matching-foreign.raw");
    let file = fs::File::create(&foreign).unwrap();
    file.set_len(16 * 1024 * 1024).unwrap();
    fs::set_permissions(&foreign, fs::Permissions::from_mode(0o600)).unwrap();
    let status = std::process::Command::new(fixture.real_mkfs)
        .args(["-q", "-F", "-b", "4096", "-U"])
        .arg(
            fixture
                .claim()
                .purpose
                .receipt()
                .intent
                .registration()
                .filesystem_uuid()
                .to_string(),
        )
        .arg(&foreign)
        .status()
        .unwrap();
    assert!(status.success(), "actual independent matching ext4 fixture");
    fs::rename(
        fixture.canonical(),
        fixture.canonical().with_extension("original"),
    )
    .unwrap();
    fs::copy(&foreign, fixture.canonical()).unwrap();
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
    assert_eq!(
        fs::read(&foreign).unwrap(),
        fs::read(fixture.canonical()).unwrap()
    );
}
#[tokio::test]
async fn hardlinked_birth_and_held_root_substitution_invalidate_custody() {
    let fixture = Fixture::new();
    prepare(&fixture, Stage::Allocated);
    let worker = Worker::new(&fixture);
    let mut guard = fixture
        .store
        .observe_owned_partial(&worker, &fixture.claim())
        .await
        .unwrap();
    let link = fixture.root.path().join("extra-link");
    fs::hard_link(fixture.canonical(), &link).unwrap();
    assert!(guard.revalidate().is_err());
    drop(guard);
    assert!(
        fixture
            .store
            .observe_owned_partial(&worker, &fixture.claim())
            .await
            .is_err()
    );
    fs::remove_file(link).unwrap();
    let mut guard = fixture
        .store
        .observe_owned_partial(&worker, &fixture.claim())
        .await
        .unwrap();
    let saved = fixture.root.path().with_extension("held-owned-saved");
    fs::rename(fixture.root.path(), &saved).unwrap();
    fs::create_dir(fixture.root.path()).unwrap();
    fs::write(fixture.root.path().join("foreign"), b"untouched").unwrap();
    let before = snapshot(fixture.root.path());
    assert!(guard.revalidate().is_err());
    assert_eq!(before, snapshot(fixture.root.path()));
    drop(guard);
    fs::remove_dir_all(fixture.root.path()).unwrap();
    fs::rename(saved, fixture.root.path()).unwrap();
}
