use runtime_authority::LiveRunSnapshotRequest;
use sqlx::PgPool;

use super::{fixture, producers, sessions};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn update_source_authority_uses_immutable_consumed_admission_not_mutable_actor(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let (e, result) = producers::update(&pool, &f, true).await;
    result.unwrap();
    let repository = fixture::repository(&pool).await;
    let request = LiveRunSnapshotRequest::pre_issuance(fixture::pins(&e));
    fixture::denied(&repository, &request).await;
    let mut tx = fixture::worker(&pool).await;
    sqlx::query("UPDATE run_vm_plan_admissions SET consumed_at=now(),consumption_kind='provision',vm_id=run_id::text,vm_provider_namespace='profile-provider',vm_host_id='host' WHERE run_id=$1").bind(e.run).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE runs SET vm_id=id::text,vm_provider_namespace='profile-provider',vm_host_id='host',vm_binding_kind='planned' WHERE id=$1").bind(e.run).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    fixture::advance(&pool, e.run, "provisioning").await;
    let snapshot = sessions::snapshot(&repository, &e).await;
    sqlx::query("UPDATE agent_updates SET actor_id=$2 WHERE id=$1")
        .bind(f.update)
        .bind(f.outsider.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sessions::allowed(&repository, &request, &snapshot).await;
    let actor: uuid::Uuid =
        sqlx::query_scalar("SELECT actor_id FROM run_vm_plan_admissions WHERE run_id=$1")
            .bind(e.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(actor, f.maintainer.as_uuid());
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.project)
        .bind(f.maintainer.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    fixture::denied(&repository, &request).await;
    // A mutable update actor with source authority cannot rescue the original issuer.
    sqlx::query("UPDATE agent_updates SET actor_id=$2 WHERE id=$1")
        .bind(f.update)
        .bind(f.owner.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    fixture::denied(&repository, &request).await;
}
