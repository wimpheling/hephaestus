struct AppStore {
    app_pool: sqlx::PgPool,
    worker_pool: sqlx::PgPool,
}

impl AppStore {
    async fn write(&self) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO projects (id) VALUES ($1)")
            .execute(&self.worker_pool)
            .await?;
        Ok(())
    }
}
