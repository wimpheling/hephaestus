//! Actual Source141 three-pool startup and zero-row metadata privilege checks.

use control_plane_postgres::{connect_app, connect_worker, verify_qualified_invocation_contract};
use sqlx::{ConnectOptions, PgPool};

#[sqlx::test(migrations = false)]
#[ignore = "requires owned disposable PostgreSQL17.10 and canonical public141"]
async fn worker_metadata_permissions_match_actual_three_pool_startup(admin: PgPool) {
    sqlx::migrate!("../../../../migrations")
        .run(&admin)
        .await
        .unwrap();
    let url = admin.connect_options().to_url_lossy().to_string();
    let worker = connect_worker(&url, 2).await.unwrap();
    let app = connect_app(&url, 2).await.unwrap();
    verify_qualified_invocation_contract(&admin, &worker, &app)
        .await
        .unwrap();
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM public.secret_runtime_mounts")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(before, 0);
    for statement in [
        "SELECT opaque_directory FROM public.secret_runtime_mounts WHERE FALSE",
        "INSERT INTO public.secret_runtime_mounts(run_id,opaque_directory,state)
         SELECT NULL::uuid,NULL::uuid,'materialized' WHERE FALSE",
        "UPDATE public.secret_runtime_mounts SET state=state WHERE FALSE",
    ] {
        allowed(&worker, statement).await;
    }
    denied(
        &worker,
        "hephaestus_worker",
        "DELETE FROM public.secret_runtime_mounts WHERE FALSE",
    )
    .await;
    for statement in [
        "SELECT opaque_directory FROM public.secret_runtime_mounts WHERE FALSE",
        "INSERT INTO public.secret_runtime_mounts(run_id,opaque_directory,state)
         SELECT NULL::uuid,NULL::uuid,'materialized' WHERE FALSE",
        "UPDATE public.secret_runtime_mounts SET state=state WHERE FALSE",
        "DELETE FROM public.secret_runtime_mounts WHERE FALSE",
    ] {
        denied(&app, "hephaestus_app", statement).await;
    }
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM public.secret_runtime_mounts")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(after, before);
    app.close().await;
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires owned disposable PostgreSQL17.10 and canonical public141"]
async fn missing_task_local_worker_metadata_update_rejects_startup(admin: PgPool) {
    sqlx::migrate!("../../../../migrations")
        .run(&admin)
        .await
        .unwrap();
    let url = admin.connect_options().to_url_lossy().to_string();
    let worker = connect_worker(&url, 2).await.unwrap();
    let app = connect_app(&url, 2).await.unwrap();
    verify_qualified_invocation_contract(&admin, &worker, &app)
        .await
        .unwrap();
    // Only this disposable SQLx fixture is modified, never a retained database.
    sqlx::query("REVOKE UPDATE ON public.secret_runtime_mounts FROM hephaestus_worker")
        .execute(&admin)
        .await
        .unwrap();
    let error = verify_qualified_invocation_contract(&admin, &worker, &app)
        .await
        .expect_err("missing ordinary metadata ACL must stop startup");
    assert!(
        error
            .to_string()
            .contains("Source141 secret mount metadata")
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM public.secret_runtime_mounts")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    app.close().await;
    worker.close().await;
}

async fn allowed(pool: &PgPool, statement: &str) {
    let mut transaction = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ WRITE")
        .execute(&mut *transaction)
        .await
        .unwrap();
    let role: (String, String, String) =
        sqlx::query_as("SELECT session_user::text,current_user::text,current_setting('role')")
            .fetch_one(&mut *transaction)
            .await
            .unwrap();
    let result = sqlx::query(statement).execute(&mut *transaction).await;
    transaction.rollback().await.unwrap();
    assert_eq!(
        role,
        (
            "postgres".into(),
            "hephaestus_worker".into(),
            "hephaestus_worker".into()
        )
    );
    assert_eq!(result.unwrap().rows_affected(), 0);
}

async fn denied(pool: &PgPool, expected_role: &str, statement: &str) {
    let mut transaction = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ WRITE")
        .execute(&mut *transaction)
        .await
        .unwrap();
    let role: (String, String, String) =
        sqlx::query_as("SELECT session_user::text,current_user::text,current_setting('role')")
            .fetch_one(&mut *transaction)
            .await
            .unwrap();
    let result = sqlx::query(statement).execute(&mut *transaction).await;
    transaction.rollback().await.unwrap();
    assert_eq!(
        role,
        (
            "postgres".into(),
            expected_role.into(),
            expected_role.into()
        )
    );
    assert_eq!(
        result
            .unwrap_err()
            .as_database_error()
            .unwrap()
            .code()
            .as_deref(),
        Some("42501")
    );
}
