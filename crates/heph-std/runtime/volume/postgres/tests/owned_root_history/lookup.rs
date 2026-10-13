use std::{os::unix::ffi::OsStrExt, path::Path};

use sqlx::PgPool;
use volume_trait::{
    OwnedVolumeRootHistory, OwnedVolumeRootHistoryConflict, VolumeError,
    VolumeRootHistoryRepository,
};

use super::support::{namespace, other_project, purpose};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private111 PostgreSQL"]
async fn no_history_is_explicit_and_actor_role_cannot_infer_absence(pool: PgPool) {
    let f = crate::backing::setup(&pool).await;
    assert_eq!(
        f.worker
            .owned_root_history("host", Path::new("/volumes"))
            .await
            .unwrap(),
        OwnedVolumeRootHistory::NoOwnedHistory
    );
    assert!(matches!(
        f.actor
            .owned_root_history("host", Path::new("/volumes"))
            .await,
        Err(VolumeError::PermissionDenied)
    ));
    let ordinary = volume_postgres::PostgresVolumeMetadataRepository::new(pool.clone());
    assert!(matches!(
        ordinary
            .owned_root_history("host", Path::new("/volumes"))
            .await,
        Err(VolumeError::PermissionDenied)
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private111 PostgreSQL"]
async fn repeated_identical_history_across_projects_remains_exact_after_revocation(pool: PgPool) {
    let f = crate::backing::setup(&pool).await;
    let second = other_project(&pool, &f).await;
    let owner = namespace();
    purpose(&f, "host", "/volumes", owner).await;
    purpose(&second, "host", "/volumes", owner).await;
    crate::backing::revoke(&f).await;
    crate::backing::revoke(&second).await;
    assert_eq!(
        f.worker
            .owned_root_history("host", Path::new("/volumes"))
            .await
            .unwrap(),
        OwnedVolumeRootHistory::Existing { namespace: owner }
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private111 PostgreSQL"]
async fn one_recorded_different_root_is_redirect_without_latest_selection(pool: PgPool) {
    let f = crate::backing::setup(&pool).await;
    purpose(&f, "host", "/original", namespace()).await;
    assert_eq!(
        f.worker
            .owned_root_history("host", Path::new("/replacement"))
            .await
            .unwrap(),
        OwnedVolumeRootHistory::Conflict {
            code: OwnedVolumeRootHistoryConflict::RootRedirect
        }
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private111 PostgreSQL"]
async fn multiple_namespaces_on_same_host_root_are_conflict(pool: PgPool) {
    let f = crate::backing::setup(&pool).await;
    purpose(&f, "host", "/volumes", namespace()).await;
    purpose(&f, "host", "/volumes", namespace()).await;
    assert_eq!(
        f.worker
            .owned_root_history("host", Path::new("/volumes"))
            .await
            .unwrap(),
        OwnedVolumeRootHistory::Conflict {
            code: OwnedVolumeRootHistoryConflict::MultipleOwnerIdentities
        }
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private111 PostgreSQL"]
async fn multiple_roots_conflict_even_when_one_matches_configured_root(pool: PgPool) {
    let f = crate::backing::setup(&pool).await;
    let owner = namespace();
    purpose(&f, "host", "/volumes", owner).await;
    purpose(&f, "host", "/other-volumes", owner).await;
    assert_eq!(
        f.worker
            .owned_root_history("host", Path::new("/volumes"))
            .await
            .unwrap(),
        OwnedVolumeRootHistory::Conflict {
            code: OwnedVolumeRootHistoryConflict::MultipleOwnerIdentities
        }
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private111 PostgreSQL"]
async fn different_hosts_legitimately_share_absolute_paths_independently(pool: PgPool) {
    let f = crate::backing::setup(&pool).await;
    let first = namespace();
    let second = namespace();
    purpose(&f, "first-host", "/volumes", first).await;
    purpose(&f, "second-host", "/volumes", second).await;
    for (host, owner) in [("first-host", first), ("second-host", second)] {
        assert_eq!(
            f.worker
                .owned_root_history(host, Path::new("/volumes"))
                .await
                .unwrap(),
            OwnedVolumeRootHistory::Existing { namespace: owner }
        );
    }
    assert_eq!(
        f.worker
            .owned_root_history("new-host", Path::new("/volumes"))
            .await
            .unwrap(),
        OwnedVolumeRootHistory::NoOwnedHistory
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private111 PostgreSQL"]
async fn invalid_bounded_host_or_lexical_utf8_root_is_rejected(pool: PgPool) {
    let f = crate::backing::setup(&pool).await;
    for host in ["", "host\n", &"x".repeat(129)] {
        assert!(matches!(
            f.worker
                .owned_root_history(host, Path::new("/volumes"))
                .await,
            Err(VolumeError::IntentConflict)
        ));
    }
    for root in [
        "relative",
        "/volumes/../other",
        "/volumes//nested",
        "/volumes/",
        "/volumes\n",
        &format!("/{}", "x".repeat(4096)),
    ] {
        assert!(matches!(
            f.worker.owned_root_history("host", Path::new(root)).await,
            Err(VolumeError::IntentConflict)
        ));
    }
    let root = Path::new(std::ffi::OsStr::from_bytes(b"/volumes/\xff"));
    assert!(matches!(
        f.worker.owned_root_history("host", root).await,
        Err(VolumeError::IntentConflict)
    ));
}
