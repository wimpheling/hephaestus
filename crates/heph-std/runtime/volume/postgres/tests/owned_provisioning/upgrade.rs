use sqlx::PgPool;
use volume_trait::{VolumeOwnedRegistrationRepository, VolumeResourceRepository};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn genuine_110_to_111_preserves_sealed_and_independent_metadata_without_backfill(
    pool: PgPool,
) {
    crate::seals::checkpoint(110).run(&pool).await.unwrap();
    let fixture = crate::seals::seed(&pool).await;
    let sealed = crate::seals::intent(&fixture);
    let receipt = fixture
        .actor
        .register_owned(&crate::seals::identity(fixture.owner), &sealed)
        .await
        .unwrap();
    let independent = crate::seals::intent(&fixture);
    fixture
        .actor
        .register(
            &crate::seals::identity(fixture.owner),
            independent.registration(),
        )
        .await
        .unwrap();
    let before: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let checksums: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    crate::seals::checkpoint(111).run(&pool).await.unwrap();
    let after: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(after, before);
    let after_checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=110 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(after_checksums, checksums);
    assert_eq!(
        fixture
            .worker
            .creation_lineage(sealed.registration().id())
            .await
            .unwrap(),
        volume_trait::VolumeCreationLineage::Owned(Box::new(receipt))
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_backing_purposes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "historical rows acquire no backing ownership evidence"
    );
}
