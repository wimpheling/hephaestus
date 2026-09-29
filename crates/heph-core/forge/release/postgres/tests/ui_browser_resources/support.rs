//! Shared database setup for browser-resource integration scenarios.

use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;
use uuid::Uuid;

const EXPECTED_MIGRATION: i64 = 98;

pub async fn connect_pools(database_url: &str) -> (i64, PgPool, PgPool) {
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0098");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker");
    assert!(max_migration >= EXPECTED_MIGRATION);
    let worker = role_pool(database_url, "hephaestus_worker").await;
    let app = role_pool(database_url, "hephaestus_app").await;
    (max_migration, worker, app)
}

pub async fn assert_app_function_contracts(app: &PgPool) {
    for function_signature in [
        "public.resolve_active_ui_generation_host(uuid)",
        "public.resolve_ui_browser_resource(bytea,uuid,text,text)",
        "public.resolve_ui_browser_repository_target_context(bytea,uuid)",
    ] {
        let (security_definer, executable, safe_search_path): (bool, bool, bool) = sqlx::query_as(
            "SELECT p.prosecdef,
                        has_function_privilege(current_user, p.oid, 'EXECUTE'),
                        EXISTS (
                            SELECT 1
                            FROM unnest(coalesce(p.proconfig, ARRAY[]::text[])) AS setting
                            WHERE setting = 'search_path=pg_catalog, pg_temp'
                        )
                 FROM pg_proc AS p
                 WHERE p.oid = to_regprocedure($1)",
        )
        .bind(function_signature)
        .fetch_one(app)
        .await
        .expect("inspect application-role UI function contract");
        assert!(
            security_definer,
            "{function_signature} must be SECURITY DEFINER"
        );
        assert!(
            executable,
            "{function_signature} must grant EXECUTE to app role"
        );
        assert!(
            safe_search_path,
            "{function_signature} must pin search_path"
        );
    }
}

async fn role_pool(database_url: &str, role: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect({
            let role = role.to_owned();
            move |connection, _metadata| {
                let role = role.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('role', $1, false)")
                        .bind(role.clone())
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('application_name', $1, false)")
                        .bind(format!("ui-browser-resource-{role}"))
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
                        .bind(Uuid::nil().to_string())
                        .execute(&mut *connection)
                        .await
                        .map(|_| ())
                })
            }
        })
        .connect(database_url)
        .await
        .expect("connect restricted PostgreSQL role")
}
