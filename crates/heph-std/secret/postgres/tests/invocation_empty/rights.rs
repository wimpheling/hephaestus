use super::fixture::Fixture;
use capability_domain::RuntimeCredentialGeneration;
use run_domain::RunState;
use run_orchestrator::RunRepository;
use runtime_authority::RuntimeSessionRepository;
use sqlx::PgPool;

async fn permission(f: &Fixture, operation: &str, kind: &str, id: uuid::Uuid) -> bool {
    sqlx::query_scalar("SELECT check_permission('user',$1::text,$2,$3,$4::text)=1")
        .bind(f.authority.birth.birth.actor.user_id.as_uuid())
        .bind(operation)
        .bind(kind)
        .bind(id)
        .fetch_one(&f.authority.birth.pool)
        .await
        .unwrap()
}
async fn withdraw_project(f: &Fixture, project: uuid::Uuid) {
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(project)
        .bind(f.authority.birth.birth.actor.user_id.as_uuid())
        .execute(&f.authority.birth.pool)
        .await
        .unwrap();
}
async fn restore_project(f: &Fixture, project: uuid::Uuid) {
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(f.authority.birth.birth.actor.user_id.as_uuid())
        .execute(&f.authority.birth.pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn source_use_loss_denies_empty_verification_before_and_after_ceiling(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let project = f.authority.birth.birth.fixture.project;
    withdraw_project(&f, project).await;
    assert!(
        !permission(
            &f,
            "can_use",
            "release_agent",
            f.authority.birth.start.release_agent_id.as_uuid()
        )
        .await
    );
    assert!(
        permission(
            &f,
            "can_execute",
            "agent_instance",
            f.authority.birth.start.instance_id.as_uuid()
        )
        .await
    );
    f.denied_unchanged().await;
    restore_project(&f, project).await;
    f.allowed_unchanged().await;
    f.authority.issue().await;
    withdraw_project(&f, project).await;
    f.denied_unchanged().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn execute_loss_denies_empty_verification_before_and_after_ceiling(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let project = f.authority.birth.birth.fixture.consuming_project;
    withdraw_project(&f, project).await;
    assert!(
        !permission(
            &f,
            "can_execute",
            "agent_instance",
            f.authority.birth.start.instance_id.as_uuid()
        )
        .await
    );
    assert!(
        permission(
            &f,
            "can_use",
            "release_agent",
            f.authority.birth.start.release_agent_id.as_uuid()
        )
        .await
    );
    f.denied_unchanged().await;
    restore_project(&f, project).await;
    f.authority.issue().await;
    withdraw_project(&f, project).await;
    f.denied_unchanged().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn capability_issuer_delegation_loss_denies_with_invoker_rights_intact(pool: PgPool) {
    let mut f = Box::pin(Fixture::new(&pool, 2)).await;
    f.authority.add_optional_bindings().await;
    f.authority.issue().await;
    sqlx::query("DELETE FROM project_capability_granters WHERE project_id=$1 AND user_id=$2")
        .bind(f.authority.birth.birth.fixture.consuming_project)
        .bind(f.authority.birth.birth.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let delegated: bool = sqlx::query_scalar(
        "SELECT can_grant_agent_capability_operations($1,'project',$2,ARRAY['inspect'])",
    )
    .bind(f.authority.birth.birth.actor.user_id.as_uuid())
    .bind(f.authority.birth.birth.fixture.consuming_project)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!delegated);
    assert!(
        permission(
            &f,
            "can_execute",
            "agent_instance",
            f.authority.birth.start.instance_id.as_uuid()
        )
        .await
    );
    assert!(
        permission(
            &f,
            "can_use",
            "release_agent",
            f.authority.birth.start.release_agent_id.as_uuid()
        )
        .await
    );
    f.denied_unchanged().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn actual_mount_revocation_blocks_empty_family_with_saved_empty_revision(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 2)).await;
    f.authority.issue().await;
    let actor = &f.authority.birth.birth.actor;
    let mut tx = authz_postgres::begin_actor_transaction(&f.authority.birth.birth.app, actor)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_instance_volume_mount_revocations(id,grant_id,created_by,request_id,authorization_model_version) SELECT $1,id,$2,$4,$5 FROM agent_instance_volume_mount_grants WHERE instance_revision_id=$3 ORDER BY slot_key LIMIT 1")
        .bind(uuid::Uuid::new_v4()).bind(actor.user_id.as_uuid())
        .bind(f.authority.birth.start.instance_revision_id.as_uuid()).bind(actor.request_id.as_uuid())
        .bind(f.authority.snapshot.authorization_model_version()).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        permission(
            &f,
            "can_execute",
            "agent_instance",
            f.authority.birth.start.instance_id.as_uuid()
        )
        .await
    );
    assert!(
        permission(
            &f,
            "can_use",
            "release_agent",
            f.authority.birth.start.release_agent_id.as_uuid()
        )
        .await
    );
    f.denied_unchanged().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn suspended_invoker_cannot_monitor_active_empty_family(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let pending = f.authority.issue().await;
    f.authority
        .repository
        .acknowledge(
            pending.id,
            RuntimeCredentialGeneration::INITIAL,
            crate::authority_harness::pg_time(),
        )
        .await
        .unwrap();
    f.authority.attach_metadata().await;
    f.authority
        .birth
        .repository()
        .transition(
            f.authority.birth.start.run_id,
            RunState::Starting,
            None,
            None,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(f.authority.birth.birth.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    f.denied_unchanged().await;
    assert!(
        f.authority
            .repository
            .find(pending.id)
            .await
            .unwrap()
            .is_some(),
        "active ceiling remains cleanup data after authority loss"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn genuine_remove_denies_empty_family_without_erasing_producer_or_fences(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    f.authority.issue().await;
    let manager = crate::support::identity(f.authority.birth.birth.fixture.outsider);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(f.authority.birth.birth.fixture.consuming_project)
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let birth = &f.authority.birth.birth;
    let snapshot = birth
        .recipe
        .inspect(&manager, birth.intent.id())
        .await
        .unwrap();
    birth
        .recipe
        .admit_remove(
            &manager,
            crate::fixtures::remove(&manager, birth.intent.id(), snapshot.version),
        )
        .await
        .unwrap();
    let immutable: bool = sqlx::query_scalar("SELECT invocation119_immutable($1)")
        .bind(f.authority.birth.start.run_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(immutable);
    f.denied_unchanged().await;
}
