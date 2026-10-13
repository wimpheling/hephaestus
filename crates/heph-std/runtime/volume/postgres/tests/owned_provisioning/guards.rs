use sqlx::PgPool;
use volume_trait::{
    VolumeMetadataRepository, VolumeOwnedProvisioningRepository, VolumeResourceRepository,
};

use super::support::{begin, identity, setup};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn missing_worker_observation_rolls_back_state_proof_and_outbox(pool: PgPool) {
    let fixture = setup(&pool).await;
    let claim = begin(&fixture).await;
    let id = claim.purpose.receipt().intent.registration().id().as_uuid();
    let worker = crate::seals::role_pool(&pool, true).await;
    let mut tx = worker.begin().await.unwrap();
    let proof = uuid::Uuid::new_v4();
    sqlx::query("SELECT set_config('hephaestus.occurrence_id',$1,true)")
        .bind(proof.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO volume_owned_transition_proofs(id,volume_id,operation_id,generation,previous_generation,mode) VALUES($1,$2,$3,1,1,'recorded')")
        .bind(proof).bind(id).bind(claim.operation_id).execute(&mut *tx).await.unwrap();
    sqlx::query(
        "UPDATE agent_instance_state_volumes SET provisioning_state='creating' WHERE id=$1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .unwrap();
    assert!(
        tx.commit().await.is_err(),
        "deferred guard requires exact observation receipt"
    );
    let state: String = sqlx::query_scalar(
        "SELECT provisioning_state FROM agent_instance_state_volumes WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, "reserved");
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_events WHERE occurrence_id=$1")
            .bind(proof)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(events, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn raw_generic_ready_generation_and_lineage_mutation_are_rejected(pool: PgPool) {
    let fixture = setup(&pool).await;
    let claim = begin(&fixture).await;
    let id = claim.purpose.receipt().intent.registration().id();
    let worker = crate::seals::role_pool(&pool, true).await;
    assert!(sqlx::query("UPDATE agent_instance_state_volumes SET state='ready',provisioning_state='ready' WHERE id=$1").bind(id.as_uuid()).execute(&worker).await.is_err());
    assert!(fixture.worker.claim_provisioning(id).await.is_err());
    assert!(
        sqlx::query(
            "UPDATE volume_owned_backing_purposes SET host_id='different' WHERE volume_id=$1"
        )
        .bind(id.as_uuid())
        .execute(&pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM agent_instance_state_volumes WHERE id=$1")
            .bind(id.as_uuid())
            .execute(&pool)
            .await
            .is_err()
    );
    let observed = fixture
        .worker
        .owned_reconciliation_context(&claim)
        .await
        .unwrap();
    assert_eq!(observed.observation, None);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn independent_metadata_cannot_supply_a_forged_owned_receipt(pool: PgPool) {
    let fixture = setup(&pool).await;
    let independent = crate::seals::intent(&fixture);
    fixture
        .actor
        .register(&identity(fixture.owner), independent.registration())
        .await
        .unwrap();
    let mut request = super::support::request(&fixture).await;
    let original = request.purpose.receipt();
    let mut forged = original.clone();
    forged.intent = independent;
    // Even a canonically well-formed hash/correlation cannot mint a committed seal.
    let actor = identity(fixture.owner);
    let encoded = forged.intent.canonical_bytes();
    let hash: Vec<u8> = sqlx::query_scalar("SELECT sha256($1::bytea)")
        .bind(encoded)
        .fetch_one(&pool)
        .await
        .unwrap();
    forged.registration_hash = hash.try_into().unwrap();
    request.purpose = volume_trait::OwnedBackingPurpose::new(
        forged,
        String::from("owned-host"),
        std::path::PathBuf::from("/owned-volumes"),
        request.purpose.owner_namespace(),
        1,
    )
    .unwrap();
    assert!(
        fixture
            .actor
            .begin_owned_provisioning(&actor, &request)
            .await
            .is_err()
    );
}
