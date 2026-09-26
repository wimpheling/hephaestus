//! Shared setup for the UI installation navigation integration test.

use sqlx::postgres::PgPoolOptions;

#[path = "assertions.rs"]
mod assertions;
#[path = "seed.rs"]
mod seed;
#[path = "seed_rows.rs"]
mod seed_rows;

pub async fn run() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping UI navigation projection: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL bootstrap pool");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations");
    let fixture = seed::seed_all(&bootstrap).await;
    let app_pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(&mut *connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect application-role pool");
    assertions::verify(&bootstrap, &app_pool, &fixture).await;
    app_pool.close().await;
    bootstrap.close().await;
}
