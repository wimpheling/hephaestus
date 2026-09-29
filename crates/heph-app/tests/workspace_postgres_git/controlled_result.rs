use super::*;

#[path = "controlled_result/fixture.rs"]
mod fixture;
use fixture::*;
#[path = "controlled_result/phases.rs"]
mod phases;
use phases::*;

#[tokio::test]
#[serial]
async fn exact_commit_becomes_one_controlled_result_with_durable_artifacts() {
    let Some(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL");
    let fixture = setup(pool).await;
    run_case(fixture).await;
}
