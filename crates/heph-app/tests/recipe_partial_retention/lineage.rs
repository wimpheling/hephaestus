use super::{
    fixture::{BirthMode, Fixture},
    host,
    ledger::Ledger,
};
use crate::coordinator::PartialRetentionError;
use recipe_application::DeploymentError;
use sqlx::PgPool;
use volume_trait::{
    OwnedBackingPurpose, OwnedVolumeRegistration, VolumeCreationIdentity, VolumeResourceRepository,
};

mod raw;

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL"]
async fn sql_shape_allows_nonderived_labels_but_trusted_preparation_rederives_and_rejects(
    pool: PgPool,
) {
    let ledger = Ledger::new(&pool).await;
    let before: serde_json::Value =
        sqlx::query_scalar("SELECT claim_json FROM recipe_effect_attempts WHERE id=$1")
            .bind(ledger.target.provenance.attempt_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    raw::insert_nonderived_labels(&ledger).await;
    assert!(matches!(
        ledger
            .recipe
            .prepare_owned_volume_creation(&ledger.identity, &ledger.target)
            .await,
        Err(DeploymentError::InputConflict)
    ));
    let after: serde_json::Value =
        sqlx::query_scalar("SELECT claim_json FROM recipe_effect_attempts WHERE id=$1")
            .bind(ledger.target.provenance.attempt_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let volumes:i64=sqlx::query_scalar("SELECT count(*) FROM agent_instance_state_volumes WHERE id=(SELECT volume_id FROM recipe_owned_volume_preparations WHERE attempt_id=$1)").bind(ledger.target.provenance.attempt_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(volumes, 0);
    let fences: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_birth_retirements")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fences, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL"]
async fn independently_registered_matching_resource_cannot_gain_preparation_or_seal(pool: PgPool) {
    let ledger = Ledger::new(&pool).await;
    let recipe_application::PlannedResourceIdentity::Volume {
        id,
        filesystem_uuid: Some(uuid),
    } = ledger.target.identity
    else {
        panic!("owned volume");
    };
    let actor = volume_postgres::PostgresVolumeMetadataRepository::new(ledger.app.clone());
    actor
        .register(
            &ledger.identity,
            &volume_trait::VolumeRegistration::new(
                id,
                ledger.seeded.consuming_project,
                16_777_216,
                uuid,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        ledger
            .recipe
            .prepare_owned_volume_creation(&ledger.identity, &ledger.target)
            .await
            .is_err()
    );
    let sealed: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT creation_seal_id FROM agent_instance_state_volumes WHERE id=$1")
            .bind(id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(sealed.is_none());
    let preparations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM recipe_owned_volume_preparations")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(preparations, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL plus native e2fsprogs"]
async fn changed_physical_claim_cannot_borrow_original_preparation_or_mutate_host(pool: PgPool) {
    let f = Fixture::new(&pool, BirthMode::AllocatedWithdrawal).await;
    let (identity, request) = f.remove().await;
    let mut forged = f.claim.clone();
    let mut receipt = forged.purpose.receipt().clone();
    let mut creation = receipt.intent.creation().fields().clone();
    creation.effect_id = uuid::Uuid::new_v4();
    receipt.intent = OwnedVolumeRegistration::new(
        receipt.intent.registration().clone(),
        VolumeCreationIdentity::try_from(creation).unwrap(),
    );
    receipt.registration_hash =
        *release_domain::ContentHash::digest(&receipt.intent.canonical_bytes()).as_bytes();
    forged.purpose = OwnedBackingPurpose::new(
        receipt,
        forged.purpose.host().into(),
        forged.purpose.root().into(),
        forged.purpose.owner_namespace(),
        forged.purpose.birth_generation(),
    )
    .unwrap();
    let before = host::tree(f.root.path());
    assert!(matches!(
        f.coordinator().retain(&identity, &request, &forged).await,
        Err(PartialRetentionError::Correlation)
    ));
    assert_eq!(host::tree(f.root.path()), before);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_partial_receipts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let fences: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_birth_retirements")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fences, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL plus native e2fsprogs"]
async fn no_host_birth_stays_held_without_claiming_absence_or_ready(pool: PgPool) {
    let f = Fixture::new(&pool, BirthMode::NoHostBirth).await;
    let (identity, request) = f.remove().await;
    let before = host::tree(f.root.path());
    assert!(
        f.coordinator()
            .retain(&identity, &request, &f.claim)
            .await
            .is_err()
    );
    assert_eq!(host::tree(f.root.path()), before);
    assert_eq!(f.format_count(), 0);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_birth_retirements")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
