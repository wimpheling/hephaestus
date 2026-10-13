//! Actual host denial controls; no fabricated worker physical observations.

use super::{
    proof,
    support::{Fixture, assert_not_ready},
};
use sqlx::PgPool;
use std::{fs, os::unix::fs::PermissionsExt};

pub async fn exercise(pool: &PgPool) {
    matching_foreign_file(pool).await;
    replaced_root(pool).await;
    contradictory_phase(pool).await;
}
async fn matching_foreign_file(pool: &PgPool) {
    let fixture = Fixture::new(pool, false).await;
    let path = fixture.request.purpose.canonical_path();
    let backing = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("foreign fixture inode");
    backing
        .set_len(
            fixture
                .request
                .purpose
                .receipt()
                .intent
                .registration()
                .capacity_bytes(),
        )
        .expect("foreign matching length");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("foreign private fixture mode");
    let status = tokio::process::Command::new(&fixture.real_mkfs)
        .args(["-q", "-F", "-b", "4096", "-U"])
        .arg(
            fixture
                .request
                .purpose
                .receipt()
                .intent
                .registration()
                .filesystem_uuid()
                .to_string(),
        )
        .arg(&path)
        .status()
        .await
        .expect("actual foreign ext4");
    assert!(status.success());
    let before = proof::digest(pool, &fs::read(&path).expect("foreign bytes")).await;
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err(),
        "matching UUID and geometry do not prove this sealed operation created the file"
    );
    assert_eq!(
        proof::digest(pool, &fs::read(path).expect("retained foreign bytes")).await,
        before
    );
    assert_eq!(
        fixture.format_count(),
        0,
        "provider must not format or adopt foreign ext4"
    );
    assert_not_ready(&fixture).await;
}
async fn replaced_root(pool: &PgPool) {
    let fixture = Fixture::new(pool, true).await;
    let claim = fixture.format_commit_gap().await;
    let preserved = fixture.tools.path().join("preserved-owned-root");
    fs::rename(fixture.root.path(), &preserved).expect("simulate configured root substitution");
    fs::create_dir(fixture.root.path()).expect("foreign replacement root");
    fs::set_permissions(fixture.root.path(), fs::Permissions::from_mode(0o700))
        .expect("private fixture mode");
    let foreign = fixture.root.path().join("foreign.txt");
    fs::write(&foreign, b"foreign root stays untouched").expect("foreign bytes");
    let before = proof::tree(pool, fixture.root.path()).await;
    assert!(fixture.store.reconcile_owned(&claim).await.is_err());
    assert_eq!(
        proof::tree(pool, fixture.root.path()).await,
        before,
        "no lock or owner metadata may be created in substituted root"
    );
    assert_eq!(fixture.format_count(), 1);
    assert_not_ready(&fixture).await;
    // Test-owned original root is retained for the fixture lifetime, not adopted.
    assert!(
        preserved
            .join(
                fixture
                    .request
                    .purpose
                    .canonical_path()
                    .file_name()
                    .expect("canonical file")
            )
            .is_file()
    );
}
async fn contradictory_phase(pool: &PgPool) {
    let fixture = Fixture::new(pool, true).await;
    let claim = fixture.format_commit_gap().await;
    let namespace = fixture
        .root
        .path()
        .join(fixture.request.purpose.namespace_name());
    fs::rename(
        namespace.join("format-intent"),
        namespace.join("format-intent.pending"),
    )
    .expect("simulate incomplete durable phase publication");
    let before = proof::tree(pool, fixture.root.path()).await;
    assert!(
        fixture.store.reconcile_owned(&claim).await.is_err(),
        "worker cannot repair contradictory host phases or mint Ready"
    );
    assert_eq!(proof::tree(pool, fixture.root.path()).await, before);
    assert_eq!(fixture.format_count(), 1);
    assert_not_ready(&fixture).await;
}
