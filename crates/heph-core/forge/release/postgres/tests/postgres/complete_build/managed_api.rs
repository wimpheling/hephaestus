use super::*;

#[path = "managed_api/collision.rs"]
mod collision;
#[path = "managed_api/rejections.rs"]
mod rejections;
#[path = "managed_api/success.rs"]
mod success;

#[tokio::test]
#[serial]
// Keep this matrix together because every case exercises the same worker-role
// CompleteBuild boundary and asserts the same atomic absence invariant.
async fn complete_build_managed_api_and_invalid_ui_matrix_is_atomic() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool_named("heph-static-install-replay").await else {
        return;
    };
    let worker_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&worker_pool)
        .await
        .expect("worker role identity");
    assert_eq!(worker_user, "hephaestus_worker");
    let worker_is_superuser: bool =
        sqlx::query_scalar("SELECT rolsuper FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&worker_pool)
            .await
            .expect("worker role superuser attribute");
    assert!(!worker_is_superuser);
    let service = ReleaseService::new(worker_pool, Arc::new(PostgresMelangeAuthorizer));

    let success_gateway = success::verify(&admin_pool, &service).await;
    collision::verify(&admin_pool, &service).await;
    rejections::verify(&admin_pool, &service, &success_gateway).await;
}
