use super::support::{Fixture, checkpoint, identity, intent, seed};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    OwnedVolumeRegistration, VolumeError, VolumeOwnedRegistrationRepository, VolumeRegistration,
    VolumeResourceRepository,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and published checkpoint 110"]
async fn genuine_109_to_110_preserves_independent_and_legacy_metadata(pool: PgPool) {
    checkpoint(109).run(&pool).await.expect("baseline 109");
    let maximum: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("baseline head");
    assert_eq!(maximum, 109);
    let fixture = seed(&pool).await;
    let independent = intent(&fixture);
    fixture
        .actor
        .register(&identity(fixture.owner), independent.registration())
        .await
        .expect("baseline independent metadata");
    let (legacy, filesystem) = seed_legacy(&fixture).await;
    let before = snapshot(&pool).await;
    let events:Value=sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(event) ORDER BY id),'[]'::jsonb) FROM application_events event")
        .fetch_one(&pool).await.expect("baseline outbox");
    let checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=109 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("baseline migration checksums");
    checkpoint(110)
        .run(&pool)
        .await
        .expect("actual populated 109 to 110 upgrade");
    assert_eq!(
        snapshot(&pool).await,
        before,
        "all old volume metadata and backing references preserved; no file bytes claimed"
    );
    let after_events:Value=sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(event) ORDER BY id),'[]'::jsonb) FROM application_events event")
        .fetch_one(&pool).await.expect("preserved outbox");
    assert_eq!(events, after_events);
    let after_checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=109 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("preserved migration checksums");
    assert_eq!(checksums, after_checksums);
    let seals: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_creation_seals")
        .fetch_one(&pool)
        .await
        .expect("no invented historical seals");
    assert_eq!(seals, 0);
    assert!(matches!(
        fixture
            .actor
            .register_owned(&identity(fixture.owner), &independent)
            .await,
        Err(VolumeError::IntentConflict)
    ));
    let proposed = OwnedVolumeRegistration::new(
        VolumeRegistration::new(
            runtime_types::VolumeId::from_uuid(legacy),
            fixture.project,
            16 * 1024 * 1024,
            filesystem,
        )
        .expect("matching legacy metadata"),
        independent.creation().clone(),
    );
    assert!(matches!(
        fixture
            .actor
            .register_owned(&identity(fixture.owner), &proposed)
            .await,
        Err(VolumeError::IntentConflict)
    ));
    let fresh = intent(&fixture);
    fixture
        .actor
        .register_owned(&identity(fixture.owner), &fresh)
        .await
        .expect("new sealed metadata after actual upgrade");
}

async fn snapshot(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(volume)-'creation_seal_id'-'creation_operation_id' ORDER BY id),'[]'::jsonb) FROM agent_instance_state_volumes volume")
        .fetch_one(pool).await.expect("complete safe test metadata snapshot")
}

async fn seed_legacy(fixture: &Fixture) -> (Uuid, Uuid) {
    let repository = Uuid::new_v4();
    let family = Uuid::new_v4();
    let instance = Uuid::new_v4();
    let volume = Uuid::new_v4();
    let filesystem = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO repositories(id,project_id,name) VALUES($1,$2,'legacy-registration-fixture')",
    )
    .bind(repository)
    .bind(fixture.project)
    .execute(&fixture.pool)
    .await
    .expect("legacy repository");
    sqlx::query("INSERT INTO agent_families(id,repository_id,agent_key) VALUES($1,$2,'legacy')")
        .bind(family)
        .bind(repository)
        .execute(&fixture.pool)
        .await
        .expect("legacy family");
    sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state,created_by) VALUES($1,$2,$3,'legacy','active',$4)")
        .bind(instance).bind(fixture.project).bind(family).bind(fixture.owner.as_uuid()).execute(&fixture.pool).await.expect("legacy instance");
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,instance_id,project_id,host_id,host_path,capacity_bytes,filesystem_uuid,state,provisioning_state,lease_generation,key_reference,encryption_version,backup_revision,checksum) VALUES($1,$2,$3,'historical-host',$4,16777216,$5,'ready','ready',17,'historical-key',2,7,'historical-backup-checksum')")
        .bind(volume).bind(instance).bind(fixture.project).bind(format!("/historical-fixture/{volume}.raw")).bind(filesystem)
        .execute(&fixture.pool).await.expect("actual baseline legacy metadata with backing references");
    sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
        .bind(instance)
        .bind(volume)
        .execute(&fixture.pool)
        .await
        .expect("legacy instance pointer");
    (volume, filesystem)
}
