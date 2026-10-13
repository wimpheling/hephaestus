use super::fixture::Fixture;
use recipe_application::{DeploymentError, PreparedInstanceSafetyRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn manager_withdrawal_and_inactive_actor_deny_without_writes(pool: PgPool) {
    let f = Fixture::new(&pool, true, true).await;
    let manager = f.manager().await;
    let command = f.admit_remove(&manager).await;
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = f.snapshot().await;
    assert!(matches!(
        f.service()
            .inspect_for_remove(&manager, command, &f.target)
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(f.snapshot().await, before);
    sqlx::query("UPDATE users SET status='active' WHERE id=$1")
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.base.fixture.consuming_project)
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = f.snapshot().await;
    assert!(matches!(
        f.service()
            .inspect_for_remove(&manager, command, &f.target)
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(f.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn role_first_facades_do_not_grant_worker_preparation_table_access(pool: PgPool) {
    let f = Fixture::new(&pool, true, true).await;
    let manager = f.manager().await;
    let command = f.admit_remove(&manager).await;
    let before = f.snapshot().await;
    let wrong = f.configured(f.base.worker.clone(), f.base.app.clone());
    assert!(matches!(
        wrong.observe_safety(&f.target).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert!(matches!(
        wrong.inspect_for_remove(&manager, command, &f.target).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    let rights:(bool,bool,bool,bool)=sqlx::query_as("SELECT has_function_privilege('hephaestus_worker','observe_prepared_instance_safety118(uuid,jsonb,jsonb,text,text)','EXECUTE'),has_function_privilege('hephaestus_app','observe_prepared_instance_safety118(uuid,jsonb,jsonb,text,text)','EXECUTE'),has_table_privilege('hephaestus_worker','recipe_prepared_instance_imports','SELECT'),has_function_privilege('hephaestus_worker','prepared_instance_target118(uuid,jsonb,jsonb)','EXECUTE')")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(rights, (true, false, false, false));
    assert_eq!(f.snapshot().await, before);
}
