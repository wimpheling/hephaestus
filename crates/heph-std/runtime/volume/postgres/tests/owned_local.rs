//! Actual private111 actor/worker `PostgreSQL` plus Local filesystem provisioning.
//! Correlation values exercise provider ownership, not a recipe engine effect claim.

#[path = "private_migrations/mod.rs"]
mod private_migrations;
#[path = "owned_registration/support.rs"]
#[allow(dead_code)] // Reuse actual actor/worker role pools and canonical seal fixtures.
mod seals;

mod owned_local {
    pub mod failures;
    pub mod proof;
    pub mod support;
    mod worker;
}

use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL, private111 and native e2fsprogs"]
async fn fresh_owned_local_birth_matches_persisted_postgres_evidence(pool: PgPool) {
    let fixture = owned_local::support::Fixture::new(&pool, false).await;
    let ready = fixture
        .store
        .provision_owned(&fixture.identity, &fixture.request)
        .await
        .expect("actual actor admission, physical birth and worker SQL Ready");
    owned_local::proof::verify(&fixture, &ready).await;
    println!(
        "OWNED_LOCAL_PG_FRESH=1 real_actor_worker_roles=1 actual_mkfs_fsck=1 exact_sql_host_birth=1 provider_boundary_not_recipe_engine_or_vm=1"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL, private111 and native e2fsprogs"]
async fn final_ready_sql_gap_recovers_readonly_after_original_actor_loss(pool: PgPool) {
    let fixture = owned_local::support::Fixture::new(&pool, true).await;
    let claim = fixture.format_commit_gap().await;
    let before = owned_local::proof::tree(&pool, fixture.root.path()).await;
    seals::revoke(&fixture.sql).await;
    assert!(
        matches!(
            fixture
                .store
                .provision_owned(&fixture.identity, &fixture.request)
                .await,
            Err(volume_trait::VolumeError::PermissionDenied)
        ),
        "historical actor admission cannot authorize replay after withdrawal"
    );
    let ready = fixture
        .store
        .reconcile_owned(&claim)
        .await
        .expect("actual worker SQL plus readonly host proof survives creator role loss");
    assert_eq!(
        owned_local::proof::tree(&pool, fixture.root.path()).await,
        before,
        "readonly reconciliation preserves all host bytes and exact inode identities"
    );
    owned_local::proof::verify(&fixture, &ready).await;
    assert_eq!(
        fixture.format_count(),
        1,
        "formatter must never repeat after a lost Ready result"
    );
    println!(
        "OWNED_LOCAL_PG_GAP=1 final_ready_record_failed_only=1 actor_withdrawal_denied=1 worker_readonly_real_fsck=1 inode_bytes_unchanged=1 formatter_count_one=1 provider_boundary_only=1"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL, private111 and native e2fsprogs"]
async fn foreign_or_substituted_birth_and_contradictory_phase_remain_held(pool: PgPool) {
    owned_local::failures::exercise(&pool).await;
    println!(
        "OWNED_LOCAL_PG_DENIALS=1 replaced_root_untouched=1 matching_foreign_uuid_not_adopted=1 readonly_contradiction_held=1 no_sql_ready=1 provider_boundary_only=1"
    );
}
