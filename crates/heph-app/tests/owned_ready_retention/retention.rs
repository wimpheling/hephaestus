use crate::{fixture::Fixture, fixtures};
use recipe_application::{InstallProgress, OwnedReadyRetentionRepository, RemovalProgress};
use sqlx::{PgPool, postgres::PgDatabaseError};
use uuid::Uuid;
use volume_trait::{VolumeOwnedProvisioningRepository, VolumeReadyRetentionRepository};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn unfinished_ready_retains_truthfully_after_original_actor_and_source_withdrawal(
    pool: PgPool,
) {
    let f = Fixture::new(&pool, false).await;
    let original = f.original().await;
    f.manager(f.birth.h.fixture.outsider).await;
    f.withdraw_installer().await;
    let (identity, request) = f.remove(f.birth.h.fixture.outsider).await;
    let bytes = std::fs::read(f.ready.claim.purpose.canonical_path()).unwrap();
    let fscks = f.birth.count("fscks");
    let progress = f
        .coordinator()
        .retain(&identity, &request, &f.ready.claim)
        .await
        .unwrap();
    assert_eq!(progress.install, InstallProgress::Ready);
    assert_eq!(progress.removal, RemovalProgress::Retained);
    assert!(progress.active_attempt.is_none());
    assert_eq!(
        f.original().await,
        original,
        "original Install evidence is never rewritten"
    );
    assert_eq!(
        std::fs::read(f.ready.claim.purpose.canonical_path()).unwrap(),
        bytes
    );
    assert_eq!(
        (f.birth.count("formats"), f.birth.count("fscks")),
        (1, fscks + 1)
    );
    let count:(i64,i64,bool)=sqlx::query_as("SELECT (SELECT count(*) FROM recipe_execution_transitions WHERE kind='creation_retained_ready'),(SELECT count(*) FROM volume_owned_partial_receipts),recipe_owned_first_birth_completed($1)")
        .bind(f.ready.claim.purpose.receipt().intent.registration().id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        count,
        (1, 0, false),
        "distinct ReadyRetention is not original Install Applied or partial"
    );
    let snapshot = f.snapshot().await;
    assert_eq!(
        f.coordinator()
            .retain(&identity, &request, &f.ready.claim)
            .await
            .unwrap(),
        progress
    );
    assert_eq!(f.snapshot().await, snapshot);
    assert_eq!(
        f.birth.count("fscks"),
        fscks + 1,
        "completed replay never probes again"
    );
    drop(f);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn permanent_closure_denies_raw_lease_and_original_ready_writes(pool: PgPool) {
    let f = Fixture::new(&pool, true).await;
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    let plan = f
        .actor
        .begin_owned_ready_retention(&identity, &request)
        .await
        .unwrap();
    let context = f
        .worker
        .ready_retention_context(&f.ready.claim, plan.closure_id)
        .await
        .unwrap();
    let before = f.snapshot().await;
    for origin in ["historical_unproven", "legacy_declaration"] {
        let error=sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,update_id,host_id,fencing_token,state,expires_at,selection_origin) VALUES($1,$2,$3,$4,'creator-fixture-host',1,'active',now()+interval '30 seconds',$5)")
            .bind(Uuid::new_v4()).bind(context.closure.fields().volume_id.as_uuid()).bind(f.birth.h.fixture.instance)
            .bind(Uuid::new_v4()).bind(origin).execute(&f.birth.h.worker).await.unwrap_err();
        let database = error
            .as_database_error()
            .unwrap()
            .downcast_ref::<PgDatabaseError>();
        assert_eq!(database.code(), "23000");
        assert_eq!(
            database.message(),
            "Ready retention admission remains closed"
        );
    }
    assert!(
        f.birth
            .worker
            .assert_owned_first_ready_verification(&f.ready.claim)
            .await
            .is_err()
    );
    let error=sqlx::query("INSERT INTO volume_owned_backing_observations(id,operation_id,volume_id,generation,birth_observation_id,phase,proof_json,evidence_hash,event_id) SELECT $1,operation_id,volume_id,generation,birth_observation_id,phase,proof_json, evidence_hash,event_id FROM volume_owned_backing_observations WHERE id=$2")
        .bind(Uuid::new_v4()).bind(context.closure.fields().ready_observation_id)
        .execute(&f.birth.h.worker).await.unwrap_err();
    let database = error
        .as_database_error()
        .unwrap()
        .downcast_ref::<PgDatabaseError>();
    assert_eq!(database.code(), "23000");
    assert_eq!(
        database.message(),
        "original owned birth is closed by Ready retention"
    );
    assert_eq!(f.snapshot().await, before);
    assert_eq!(f.birth.count("formats"), 1);
    assert_eq!(request.target.resource, fixtures::key("data"));
    drop(f);
}
