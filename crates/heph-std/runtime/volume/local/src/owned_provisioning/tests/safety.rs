use super::support::Fixture;
use std::{
    fs,
    os::unix::fs::{FileExt, PermissionsExt},
    process::Command,
};

#[tokio::test]
async fn missing_or_wrong_role_composition_rejects_before_backing_effects() {
    let fixture = Fixture::new();
    let count = fs::read_dir(fixture.root.path()).expect("entries").count();
    assert!(
        fixture
            .uncomposed()
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    assert_eq!(fixture.state.lock().expect("state").begins, 0);
    assert!(
        fixture
            .wrong_worker_store()
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    assert_eq!(
        fs::read_dir(fixture.root.path()).expect("entries").count(),
        count
    );
    assert_eq!(fixture.format_count(), 0);
}
#[tokio::test]
async fn matching_uuid_and_capacity_foreign_ext4_never_becomes_owned() {
    let fixture = Fixture::new();
    let path = fixture.canonical();
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .expect("independent inode");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("mode");
    file.set_len(16 * 1024 * 1024).expect("capacity");
    assert!(
        Command::new(fixture.real_mkfs)
            .args(["-q", "-F", "-b", "4096", "-U"])
            .arg(
                fixture
                    .request
                    .purpose
                    .receipt()
                    .intent
                    .registration()
                    .filesystem_uuid()
                    .to_string()
            )
            .arg(&path)
            .status()
            .expect("real independent mkfs")
            .success()
    );
    let bytes = fs::read(&path).expect("foreign bytes");
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(fs::read(&path).expect("retained"), bytes);
    assert_eq!(fixture.format_count(), 0);
    assert!(
        !fixture
            .root
            .path()
            .join(fixture.request.purpose.namespace_name())
            .exists()
    );
}
#[tokio::test]
async fn substituted_root_is_rejected_without_mutating_foreign_directory() {
    let fixture = Fixture::new();
    let retained = fixture.tools.path().join("retained-volume-root");
    fs::rename(fixture.root.path(), &retained).expect("replace");
    fs::create_dir(fixture.root.path()).expect("foreign root");
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(
        fs::read_dir(fixture.root.path())
            .expect("foreign entries")
            .count(),
        0
    );
    assert_eq!(fixture.state.lock().expect("state").begins, 0);
    assert_eq!(fixture.format_count(), 0);
}
#[tokio::test]
async fn ambiguous_formatter_failure_never_permits_another_format() {
    let fixture = Fixture::new();
    fixture.formatter("exit 1\n");
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    let before = fs::read(fixture.canonical()).expect("retained incomplete birth");
    fixture.formatter(&format!("exec '{}' \"$@\"\n", fixture.real_mkfs));
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(fixture.format_count(), 1);
    assert_eq!(fs::read(fixture.canonical()).expect("retained"), before);
    assert_eq!(fixture.claim().generation, 1);
}
#[tokio::test]
async fn dirty_initial_birth_is_held_by_readonly_worker_without_reformat() {
    let fixture = Fixture::new();
    fixture.state.lock().expect("state").fail_ready_once = true;
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    let file = fs::OpenOptions::new()
        .write(true)
        .open(fixture.canonical())
        .expect("dirty fault injection");
    file.write_all_at(&2_u16.to_le_bytes(), 1024 + 58)
        .expect("dirty ext4 state");
    let before = fs::read(fixture.canonical()).expect("dirty bytes");
    fixture.state.lock().expect("state").authorized = false;
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(fs::read(fixture.canonical()).expect("retained"), before);
    assert_eq!(fixture.format_count(), 1);
}
#[tokio::test]
async fn fresh_actor_revocation_after_allocation_prevents_first_format() {
    let fixture = Fixture::new();
    fixture.state.lock().expect("state").reject_begin = Some(3);
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    assert!(fixture.canonical().exists());
    assert_eq!(fixture.format_count(), 0);
    assert!(
        !fixture
            .root
            .path()
            .join(fixture.request.purpose.namespace_name())
            .join("format-intent")
            .exists()
    );
    fixture.state.lock().expect("state").authorized = false;
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(fixture.format_count(), 0);
}
#[tokio::test]
async fn readonly_worker_does_not_create_missing_lock_or_namespace() {
    let fixture = Fixture::new();
    let names = fs::read_dir(fixture.root.path())
        .expect("entries")
        .map(|entry| entry.expect("entry").file_name())
        .collect::<Vec<_>>();
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(
        fs::read_dir(fixture.root.path())
            .expect("entries")
            .map(|entry| entry.expect("entry").file_name())
            .collect::<Vec<_>>(),
        names
    );
    assert_eq!(fixture.state.lock().expect("state").begins, 0);
}

#[test]
fn readonly_recovery_descriptor_cannot_write_even_before_format_intent() {
    let fixture = Fixture::new();
    let mut lock =
        crate::owned_journal::OwnedJournalLock::acquire(fixture.journal_purpose()).expect("lock");
    let mut journal = crate::owned_journal::OwnedJournal::open(&mut lock).expect("journal");
    let backing = journal.create_staging().expect("birth");
    let readonly = journal
        .recover_readonly()
        .expect("readonly birth descriptor");
    assert!(readonly.file().write_at(b"cannot-write", 0).is_err());
    assert_eq!(backing.file().metadata().expect("metadata").len(), 0);
}

#[test]
fn replacement_between_owner_prevalidation_and_journal_open_never_creates_foreign_metadata() {
    let fixture = Fixture::new();
    let (pinned, identity) = fixture
        .owner
        .pinned_root()
        .expect("prevalidated owner root");
    let retained = fixture.tools.path().join("retained-prevalidated-root");
    fs::rename(fixture.root.path(), &retained).expect("root substitution after validation");
    fs::create_dir(fixture.root.path()).expect("replacement directory");
    fs::write(
        fixture.root.path().join("foreign-retained"),
        b"foreign bytes",
    )
    .expect("foreign bytes");
    for existing in [false, true] {
        assert!(
            crate::owned_journal::OwnedJournalLock::acquire_pinned(
                &pinned,
                identity,
                fixture.journal_purpose(),
                existing
            )
            .is_err()
        );
    }
    assert_eq!(
        fs::read_dir(fixture.root.path())
            .expect("foreign entries")
            .count(),
        1
    );
    assert_eq!(
        fs::read(fixture.root.path().join("foreign-retained")).expect("retained foreign"),
        b"foreign bytes"
    );
    assert!(fixture.owner.validate().is_err());
    assert_eq!(
        fs::read_dir(retained).expect("original entries").count(),
        1,
        "only original owner marker retained; no journal acquired"
    );
}

#[tokio::test]
async fn revocation_during_worker_format_intent_record_prevents_actual_formatter_spawn() {
    let fixture = Fixture::new();
    fixture.state.lock().expect("state").revoke_on_format_record = true;
    assert!(matches!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await,
        Err(volume_trait::VolumeError::PermissionDenied)
    ));
    let context = fixture.state.lock().expect("state").context.clone();
    assert_eq!(
        context
            .observation
            .expect("durable intent proof")
            .fields()
            .phase,
        volume_trait::OwnedBackingPhase::FormatIntent
    );
    assert!(
        fixture
            .root
            .path()
            .join(fixture.request.purpose.namespace_name())
            .join("format-intent")
            .exists()
    );
    assert_eq!(
        fixture.format_count(),
        0,
        "worker record barrier revoked authority before spawn"
    );
    let bytes = fs::read(fixture.canonical()).expect("unformatted retained birth");
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    fixture.state.lock().expect("state").authorized = true;
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    assert_eq!(
        fixture.format_count(),
        0,
        "durable format intent remains held even after authority returns"
    );
    assert_eq!(fs::read(fixture.canonical()).expect("retained"), bytes);
}
