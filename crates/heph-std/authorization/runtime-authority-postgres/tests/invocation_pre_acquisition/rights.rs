use super::fixture::Fixture;
use sqlx::PgPool;

pub async fn caller_rights(f: &Fixture) -> (bool, bool) {
    sqlx::query_as("SELECT check_permission('user',$1::text,'can_execute','agent_instance',$2::text)=1,check_permission('user',$1::text,'can_use','release_agent',$3::text)=1")
        .bind(f.birth.birth.actor.user_id.as_uuid()).bind(f.run.instance_id.as_uuid())
        .bind(f.run.release_agent_id.as_uuid()).fetch_one(&f.birth.pool).await.unwrap()
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn current_source_execute_and_saved_invoker_status_withdrawal_deny(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    for (project, expected) in [
        (f.birth.birth.fixture.project, (true, false)),
        (f.birth.birth.fixture.consuming_project, (false, true)),
    ] {
        f.allowed().await;
        sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
            .bind(project)
            .bind(f.birth.birth.actor.user_id.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(caller_rights(&f).await, expected);
        f.denied(&f.run).await;
        sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
            .bind(project)
            .bind(f.birth.birth.actor.user_id.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
    }
    f.allowed().await;
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(f.birth.birth.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    f.denied(&f.run).await;
}
