use sqlx::PgPool;
use uuid::Uuid;

pub async fn build_count(pool: &PgPool, repository: Uuid, commit: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM build_requests WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository)
    .bind(commit)
    .fetch_one(pool)
    .await
    .expect("build count")
}

pub async fn outbox_count(pool: &PgPool, commit: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM outbox WHERE payload->>'source_commit' = $1")
        .bind(commit)
        .fetch_one(pool)
        .await
        .expect("outbox count")
}

pub async fn outbox_count_for_build(pool: &PgPool, build_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
         FROM outbox
         WHERE payload->>'build_request_id' = $1",
    )
    .bind(build_id.to_string())
    .fetch_one(pool)
    .await
    .expect("outbox count for build")
}
