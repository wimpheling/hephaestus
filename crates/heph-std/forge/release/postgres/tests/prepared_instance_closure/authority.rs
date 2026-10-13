use super::fixture::Closure;
use recipe_application::{
    DeploymentError, PreparedInstanceClosureRepository, PreparedInstanceClosureRequest,
    PreparedInstanceSafetyRepository,
};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn inactive_current_manager_and_wrong_pool_deny_before_mutation(pool: PgPool) {
    let f = Closure::new(&pool, true).await;
    let request = f.request().await;
    let wrong = f
        .original
        .configured(f.original.base.worker.clone(), f.original.base.app.clone());
    let rights: (bool, bool, bool, bool) = sqlx::query_as("SELECT has_function_privilege('hephaestus_app','load_prepared_original_target118(uuid,uuid,text,uuid,bytea)','EXECUTE'),has_function_privilege('hephaestus_worker','load_prepared_original_target118(uuid,uuid,text,uuid,bytea)','EXECUTE'),has_function_privilege('hephaestus_worker','prepare_prepared_instance_closure118(uuid,jsonb,jsonb,bytea,jsonb,bytea,uuid,bigint,bytea)','EXECUTE'),has_table_privilege('hephaestus_worker','recipe_prepared_instance_imports','SELECT')")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(rights, (true, false, false, false));
    let before = f.snapshot().await;
    assert!(matches!(
        wrong.close_for_remove(&f.manager, &request).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(f.snapshot().await, before);
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(f.manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = f.snapshot().await;
    assert!(matches!(
        f.original
            .service()
            .close_for_remove(&f.manager, &request)
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert!(matches!(
        f.original
            .service()
            .load_original_target(
                &f.manager,
                f.command,
                &f.original.base.intent,
                f.original.base.input.resource()
            )
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(f.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn changed_drain_cas_or_instance_version_never_writes_partial_closure(pool: PgPool) {
    let f = Closure::new(&pool, true).await;
    let request = f.request().await;
    let mut drain = request.drain().clone();
    drain.resource_version += 1;
    let wrong = PreparedInstanceClosureRequest::new(
        request.intent().clone(),
        request.target().clone(),
        drain,
        request.expected_instance_version(),
    )
    .unwrap();
    let before = f.snapshot().await;
    assert!(matches!(
        f.original
            .service()
            .close_for_remove(&f.manager, &wrong)
            .await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    let stale = PreparedInstanceClosureRequest::new(
        request.intent().clone(),
        request.target().clone(),
        request.drain().clone(),
        request.expected_instance_version() + 1,
    )
    .unwrap();
    assert!(matches!(
        f.original
            .service()
            .close_for_remove(&f.manager, &stale)
            .await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    assert_eq!(f.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn foreign_scope_and_configuration_drift_do_not_reinterpret_original(pool: PgPool) {
    let f = Closure::new(&pool, true).await;
    let request = f.request().await;
    let scope = vm_trait::VmProviderOwnerScope::new(
        "another-owner".to_owned(),
        "actual-fixture-host".to_owned(),
    )
    .unwrap();
    let service = release_postgres::PostgresPreparedInstanceSafetyRepository::new(
        f.original.base.app.clone(),
        f.original.base.worker.clone(),
        &scope,
        f.original.base.input.source().platform().clone(),
    );
    let before = f.snapshot().await;
    assert!(matches!(
        service.close_for_remove(&f.manager, &request).await,
        Err(DeploymentError::IntentMismatch)
    ));
    let mut original = request.target().original().clone();
    original.resource_version += 1;
    let contradictory = f.original.changed(original);
    let contradictory = PreparedInstanceClosureRequest::new(
        request.intent().clone(),
        contradictory,
        request.drain().clone(),
        request.expected_instance_version(),
    )
    .unwrap();
    assert!(matches!(
        f.original
            .service()
            .close_for_remove(&f.manager, &contradictory)
            .await,
        Err(DeploymentError::IntentMismatch)
    ));

    let platform = recipe_application::PlatformPolicyObservation::new(
        f.original.base.input.source().platform().policy().clone(),
        "changed/platform".to_owned(),
    )
    .unwrap();
    let exact = vm_trait::VmProviderOwnerScope::new(
        "opaque-provider-owner".to_owned(),
        "actual-fixture-host".to_owned(),
    )
    .unwrap();
    let service = release_postgres::PostgresPreparedInstanceSafetyRepository::new(
        f.original.base.app.clone(),
        f.original.base.worker.clone(),
        &exact,
        platform,
    );
    assert!(matches!(
        service.close_for_remove(&f.manager, &request).await,
        Err(DeploymentError::IntentMismatch)
    ));
    assert_eq!(f.snapshot().await, before);
}
