use super::fixture::Fixture;
use sqlx::PgPool;
use volume_trait::RunVolumeMetadataRepository;

async fn positive(pool: &PgPool, count: usize) {
    let f = Box::pin(Fixture::new(pool, count)).await;
    let before = f.rows().await;
    for _ in 0..2 {
        f.allowed().await;
    }
    let stored: (bool,i64,i64,i64,i64)=sqlx::query_as("SELECT volume_selection_snapshot IS NULL,(SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=r.id),(SELECT count(*) FROM run_authorization_snapshots WHERE run_id=r.id OR id=r.id),(SELECT count(*) FROM runtime_authority_sessions WHERE run_id=r.id OR id=r.id),(SELECT count(*) FROM agent_capability_bindings WHERE instance_revision_id=r.instance_revision_id) FROM runs r WHERE id=$1")
        .bind(f.run.id.as_uuid()).fetch_one(pool).await.unwrap();
    assert_eq!(stored, (true, 0, 0, 0, 2));
    let selections = f
        .birth
        .volumes()
        .load_run_selections(f.run.id)
        .await
        .unwrap();
    assert_eq!(selections.selections().len(), count);
    assert_eq!(f.rows().await, before);
    let old_b1: bool = sqlx::query_scalar("SELECT invocation119_authority_eligible($1,$2,false)")
        .bind(f.run.id.as_uuid())
        .bind(f.config())
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(
        !old_b1,
        "B1 still requires the real frozen set, even zero slots"
    );
    assert!(
        !f.b2(&f.run, "leasing_volume").await,
        "B2 still denies Leasing"
    );
    for instance in [f.run.instance_id.as_uuid(), f.birth.birth.fixture.instance] {
        let named: bool = sqlx::query_scalar("SELECT named_volume_dispatch_supported($1)")
            .bind(instance)
            .fetch_one(pool)
            .await
            .unwrap();
        assert!(!named);
    }
    f.birth.physical.assert_unchanged();
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn genuine_zero_volume_first_acquisition_is_repeatable_and_read_only(pool: PgPool) {
    positive(&pool, 0).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn genuine_two_volume_first_acquisition_checks_full_mount_set_read_only(pool: PgPool) {
    positive(&pool, 2).await;
}
