use super::support::{identity, intent, role_pool, setup};
use authz_postgres::begin_actor_transaction;
use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{VolumeOwnedRegistrationRepository, VolumeResourceRepository};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and published checkpoint 110"]
async fn raw_promotion_and_lineage_mutation_are_rejected(pool: PgPool) {
    let fixture = setup(&pool).await;
    let independent = intent(&fixture);
    fixture
        .actor
        .register(&identity(fixture.owner), independent.registration())
        .await
        .expect("independent metadata");
    assert!(sqlx::query("UPDATE agent_instance_state_volumes SET creation_seal_id=$2,creation_operation_id=$3 WHERE id=$1")
        .bind(independent.registration().id().as_uuid()).bind(Uuid::new_v4()).bind(Uuid::new_v4())
        .execute(&pool).await.is_err(),"even privileged SQL cannot promote a historical row");
    let owned = intent(&fixture);
    let receipt = fixture
        .actor
        .register_owned(&identity(fixture.owner), &owned)
        .await
        .expect("owned metadata");
    assert!(
        sqlx::query("UPDATE volume_creation_seals SET generation=generation+1 WHERE id=$1")
            .bind(receipt.seal_id.as_uuid())
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM volume_creation_seals WHERE id=$1")
            .bind(receipt.seal_id.as_uuid())
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query(
            "INSERT INTO volume_creation_seals SELECT * FROM volume_creation_seals WHERE id=$1"
        )
        .bind(receipt.seal_id.as_uuid())
        .execute(&pool)
        .await
        .is_err(),
        "privileged caller cannot forge a seal insertion"
    );
    let role_pool = role_pool(&pool, true).await;
    assert!(
        sqlx::query(
            "INSERT INTO volume_creation_seals SELECT * FROM volume_creation_seals WHERE id=$1"
        )
        .bind(receipt.seal_id.as_uuid())
        .execute(&role_pool)
        .await
        .is_err(),
        "worker has read authority only"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and published checkpoint 110"]
async fn missing_seal_rolls_back_volume_and_creation_outbox(pool: PgPool) {
    let fixture = setup(&pool).await;
    let intent = intent(&fixture);
    let actor_pool = role_pool(&pool, false).await;
    let actor = identity(fixture.owner);
    let mut tx = begin_actor_transaction(&actor_pool, &actor)
        .await
        .expect("actor transaction");
    sqlx::query("SELECT set_config('hephaestus.occurrence_id',$1,true)")
        .bind(
            intent
                .creation()
                .fields()
                .operation_id
                .as_uuid()
                .to_string(),
        )
        .execute(&mut *tx)
        .await
        .expect("operation provenance");
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,state,capacity_bytes,filesystem_uuid,creation_seal_id,creation_operation_id) VALUES($1,$2,'uninitialized',$3,$4,$5,$6)")
        .bind(intent.registration().id().as_uuid()).bind(fixture.project).bind(16_i64*1024*1024)
        .bind(intent.registration().filesystem_uuid()).bind(Uuid::new_v4()).bind(intent.creation().fields().operation_id.as_uuid())
        .execute(&mut *tx).await.expect("provisional row and creation event");
    assert!(
        tx.commit().await.is_err(),
        "deferred exact birth seal is mandatory at commit"
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_instance_state_volumes WHERE id=$1")
            .bind(intent.registration().id().as_uuid())
            .fetch_one(&pool)
            .await
            .expect("rolled back metadata");
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_events WHERE occurrence_id=$1")
            .bind(intent.creation().fields().operation_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("rolled back outbox");
    assert_eq!((rows, events), (0, 0));
}
