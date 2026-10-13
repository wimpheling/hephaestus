use super::support::{begin, observation, partial};
use sqlx::PgPool;
use volume_trait::{
    OwnedBackingPhase, OwnedPartialBirthPhase, VolumeOwnedProvisioningRepository,
    VolumePartialRetentionRepository,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private111->112 chain"]
async fn genuine_111_upgrade_preserves_history_without_guessing_current_head(pool: PgPool) {
    crate::seals::checkpoint(111).run(&pool).await.unwrap();
    let f = crate::seals::seed(&pool).await;
    let claim = begin(&f).await;
    let recorded = observation(&claim, OwnedBackingPhase::Recorded);
    f.worker
        .record_owned_observation(&claim, &recorded)
        .await
        .unwrap();
    let before:serde_json::Value=sqlx::query_scalar("SELECT jsonb_build_array(to_jsonb(volume),to_jsonb(purpose),to_jsonb(operation),to_jsonb(observation)) FROM agent_instance_state_volumes volume JOIN volume_owned_backing_purposes purpose ON purpose.volume_id=volume.id JOIN volume_owned_provisioning_operations operation ON operation.volume_id=volume.id JOIN volume_owned_backing_observations observation ON observation.volume_id=volume.id WHERE volume.id=$1")
        .bind(claim.purpose.receipt().intent.registration().id().as_uuid()).fetch_one(&pool).await.unwrap();
    let checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=111 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    crate::seals::checkpoint(112).run(&pool).await.unwrap();
    let after:serde_json::Value=sqlx::query_scalar("SELECT jsonb_build_array(to_jsonb(volume),to_jsonb(purpose),to_jsonb(operation),to_jsonb(observation)) FROM agent_instance_state_volumes volume JOIN volume_owned_backing_purposes purpose ON purpose.volume_id=volume.id JOIN volume_owned_provisioning_operations operation ON operation.volume_id=volume.id JOIN volume_owned_backing_observations observation ON observation.volume_id=volume.id WHERE volume.id=$1")
        .bind(claim.purpose.receipt().intent.registration().id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);
    let preserved: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=111 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(checksums, preserved);
    let ctx = f.worker.partial_retention_context(&claim).await.unwrap();
    assert_eq!(
        ctx.head.version, 0,
        "historic rows cannot be guessed into a head"
    );
    assert!(ctx.head.observation_id.is_none());
    f.worker
        .record_partial_retention_observation(
            &ctx,
            &partial(&ctx, OwnedPartialBirthPhase::AllocatedNeverFormat),
        )
        .await
        .unwrap();
    assert_eq!(
        f.worker
            .partial_retention_context(&claim)
            .await
            .unwrap()
            .head
            .version,
        1
    );
}
