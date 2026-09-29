struct AppStore {
    app_pool: sqlx::PgPool,
}

impl AppStore {
    async fn read(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT id FROM projects")
            .fetch_all(&self.app_pool)
            .await?;
        Ok(())
    }
}
