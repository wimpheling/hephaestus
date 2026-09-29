pub async fn load(pool: &sqlx::PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT 1").fetch_one(pool).await
}
