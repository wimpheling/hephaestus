//! Shared setup for release UI authorization coverage.

use sqlx::postgres::PgPoolOptions;

#[path = "assertions.rs"]
mod assertions;
#[path = "helpers.rs"]
mod helpers;
#[path = "seed.rs"]
mod seed;

pub async fn run() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping release UI authorization: test URL is unset");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(12)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply migrations");
    let scenarios = seed::seed_all(&pool).await;
    let app_pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect application-role PostgreSQL pool");
    assertions::verify(&app_pool, &scenarios).await;
    app_pool.close().await;
    pool.close().await;
}
