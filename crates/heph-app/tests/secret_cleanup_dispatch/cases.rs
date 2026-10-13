use crate::{
    fixture::Fixture,
    ports::{self, CountedProvider},
};
use heph_secret::{RawSecretFile, SecretMountMetadata, SecretMountProvider};
use run_domain::RunKind;
use run_orchestrator::RunSecretManager;
use secret_domain::{SecretSlotKey, SecretValue};
use secret_postgres::PostgresSecretMountMetadata;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{os::unix::fs::PermissionsExt, sync::Arc};

const SLOT: &str = "ordinary_cleanup_fixture";
// Public fixture contents only; no resolver, decryption or credential is involved.
const CONTENT: &[u8] = b"ordinary cleanup fixture\n";

async fn worker_denied(
    f: &Fixture,
    metadata: &PostgresSecretMountMetadata,
    manager: &ports::Manager,
    provider: &CountedProvider,
) {
    let rows = f.rows().await;
    let files = f.files();
    let calls = provider.calls();
    for error in [
        metadata.materialized_directory(f.run).await.unwrap_err(),
        manager.destroy_after_guest(f.run).await.unwrap_err(),
    ] {
        assert_eq!(
            error.to_string(),
            "run secret operation failed: secret runtime persistence failed"
        );
    }
    let mut tx = f.worker.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .unwrap();
    let kind: String = sqlx::query_scalar("SELECT run_kind FROM runs WHERE id=$1")
        .bind(f.run.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(kind, f.kind);
    let error = sqlx::query("SELECT opaque_directory FROM secret_runtime_mounts WHERE run_id=$1 AND state='materialized'")
        .bind(f.run.as_uuid()).fetch_all(&mut *tx).await.unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    assert_eq!(f.rows().await, rows);
    assert_eq!(f.files(), files);
    assert_eq!(provider.calls(), calls);
}

fn assert_materialized_files(
    f: &Fixture,
    mounted: &heph_secret::MaterializedSecretMount,
    provider: &CountedProvider,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let directory = f
        .root
        .path()
        .join(mounted.opaque_directory.simple().to_string());
    let file = directory.join(SLOT);
    assert_eq!(std::fs::read(&file).unwrap(), CONTENT);
    assert_eq!(
        Sha256::digest(std::fs::read(&file).unwrap()),
        Sha256::digest(CONTENT)
    );
    assert_eq!(
        directory.metadata().unwrap().permissions().mode() & 0o777,
        0o500
    );
    assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o400);
    assert_eq!(
        f.files().len(),
        3,
        "one actual opaque directory and public fixture file"
    );
    assert_eq!(
        provider.calls(),
        vec!["validate", "validate", "materialize"]
    );
    (directory, file)
}

async fn cleanup_and_replay(
    f: &Fixture,
    owner: &PostgresSecretMountMetadata,
    manager: &ports::Manager,
    provider: &CountedProvider,
    paths: (&std::path::Path, &std::path::Path),
    materialized: &serde_json::Value,
    initial_files: &crate::fixture::Files,
) {
    let (directory, file) = paths;
    manager.destroy_after_guest(f.run).await.unwrap();
    assert!(!directory.exists() && !file.exists());
    assert_eq!(f.files(), *initial_files);
    let destroyed = f.rows().await;
    Fixture::assert_destroyed(materialized, &destroyed);
    assert_eq!(
        provider.calls(),
        vec!["validate", "validate", "materialize", "destroy"]
    );
    assert_eq!(owner.materialized_directory(f.run).await.unwrap(), None);
    manager.destroy_after_guest(f.run).await.unwrap();
    assert_eq!(f.rows().await, destroyed);
    assert_eq!(f.files(), *initial_files);
    assert_eq!(
        provider.calls(),
        vec!["validate", "validate", "materialize", "destroy"]
    );
    f.verify_public().await;
    f.worker.close().await;
}

async fn exercise(pool: PgPool, kind: RunKind, configured: bool) {
    let f = Fixture::new(&pool, kind, configured).await;
    let owner = f.metadata(f.pool.clone());
    let worker = f.metadata(f.worker.clone());
    let provider = Arc::new(CountedProvider::default());
    let manager = ports::manager(owner.clone(), Arc::clone(&provider), f.config());
    let worker_manager = ports::manager(worker.clone(), Arc::clone(&provider), f.config());
    let initial_rows = f.rows().await;
    let initial_files = f.files();
    assert_eq!(initial_files.len(), 1, "initial physical root has no mount");
    assert_eq!(owner.materialized_directory(f.run).await.unwrap(), None);
    manager.destroy_after_guest(f.run).await.unwrap();
    assert_eq!(f.rows().await, initial_rows);
    assert_eq!(f.files(), initial_files);
    assert_eq!(provider.calls(), vec!["validate", "validate"]);
    worker_denied(&f, &worker, &worker_manager, &provider).await;

    let mounted = provider
        .materialize(
            &f.config(),
            f.run,
            vec![RawSecretFile {
                slot: SecretSlotKey::parse(SLOT).unwrap(),
                value: SecretValue::new(CONTENT.to_vec()).unwrap(),
            }],
            None,
        )
        .unwrap();
    let (directory, file) = assert_materialized_files(&f, &mounted, &provider);
    owner
        .persist_mount(f.run, mounted.opaque_directory)
        .await
        .unwrap();
    assert_eq!(
        owner.materialized_directory(f.run).await.unwrap(),
        Some(mounted.opaque_directory)
    );
    let materialized = f.rows().await;
    f.assert_created(&initial_rows, &materialized, mounted.opaque_directory);
    worker_denied(&f, &worker, &worker_manager, &provider).await;

    cleanup_and_replay(
        &f,
        &owner,
        &manager,
        &provider,
        (&directory, &file),
        &materialized,
        &initial_files,
    )
    .await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires native PostgreSQL public107 and a private tmpfs root; real filesystem cleanup"]
async fn ordinary_normal_default_destroy_uses_original_mount_io(pool: PgPool) {
    exercise(pool, RunKind::Normal, false).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires native PostgreSQL public107 and a private tmpfs root; real filesystem cleanup"]
async fn ordinary_normal_configured_destroy_requires_no_private_schema(pool: PgPool) {
    exercise(pool, RunKind::Normal, true).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires native PostgreSQL public107 and a private tmpfs root; real filesystem cleanup"]
async fn ordinary_update_default_destroy_uses_original_mount_io(pool: PgPool) {
    exercise(pool, RunKind::Update, false).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires native PostgreSQL public107 and a private tmpfs root; real filesystem cleanup"]
async fn ordinary_update_configured_destroy_requires_no_private_schema(pool: PgPool) {
    exercise(pool, RunKind::Update, true).await;
}
