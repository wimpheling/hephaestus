pub async fn load(
    pool: &sqlx::PgPool,
    name: &str,
    after_name: &str,
    after_id: i64,
    limit: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT items.id
           FROM items
          WHERE items.name = $1
            AND (items.name, items.id) > ($2, $3)
          ORDER BY items.name ASC, items.id ASC
          LIMIT $4",
    )
    .bind(name)
    .bind(after_name)
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}
