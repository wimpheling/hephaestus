use super::fixture::{Fixture, code};
use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn current_mount_source_issuer_revocation_and_whole_set_loss_deny(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 2)).await;
    f.allowed().await;
    let selections = f
        .birth
        .volumes()
        .load_run_selections(f.run.id)
        .await
        .unwrap();
    assert_eq!(selections.selections().len(), 2);
    let volume = selections.selections()[1].scope().volume_id().as_uuid();
    let project = f.birth.birth.fixture.consuming_project;
    let actor = f.birth.birth.actor.user_id.as_uuid();
    // The real mount issuer's source and caller Execute share this project.
    // Assert actual source denial; do not call it an isolated mount-only loss.
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(project)
        .bind(actor)
        .execute(&pool)
        .await
        .unwrap();
    let source: bool = sqlx::query_scalar("SELECT private_volume_mount_source_is_live($1,$2)")
        .bind(actor)
        .bind(volume)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!source);
    f.denied(&f.run).await;
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(actor)
        .execute(&pool)
        .await
        .unwrap();
    f.allowed().await;
    // Sealed backing capacity cannot be changed through the actual111 guards.
    let before = f.rows().await;
    let error = sqlx::query(
        "UPDATE agent_instance_state_volumes SET capacity_bytes=capacity_bytes-4096 WHERE id=$1",
    )
    .bind(volume)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"));
    assert_eq!(f.rows().await, before);
    f.allowed().await;
    // Revoking only the second real grant makes the whole selection ineligible.
    let mut tx = begin_actor_transaction(&f.birth.birth.app, &f.birth.birth.actor)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_instance_volume_mount_revocations(id,grant_id,created_by,request_id,authorization_model_version) SELECT $1,id,$2,$4,$5 FROM agent_instance_volume_mount_grants WHERE instance_revision_id=$3 AND volume_id=$6")
        .bind(Uuid::new_v4()).bind(actor).bind(f.run.instance_revision_id.as_uuid())
        .bind(f.birth.birth.actor.request_id.as_uuid()).bind(AUTHORIZATION_MODEL_VERSION).bind(volume)
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(super::rights::caller_rights(&f).await, (true, true));
    let grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_instance_volume_mount_grants WHERE instance_revision_id=$1",
    )
    .bind(f.run.instance_revision_id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(grants, 2);
    let remaining:i64=sqlx::query_scalar("SELECT count(*) FROM agent_instance_volume_mount_grants g WHERE instance_revision_id=$1 AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations WHERE grant_id=g.id)")
        .bind(f.run.instance_revision_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        remaining, 1,
        "one of the full two-grant set remains unrevoked"
    );
    f.denied(&f.run).await;
}
