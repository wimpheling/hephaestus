struct AppStore {
    app_pool: sqlx::PgPool,
}

impl AppStore {
    async fn read(&self) -> Result<(), sqlx::Error> {
        let mut transaction = self.app_pool.begin().await?;
        sqlx::query("SELECT id FROM projects")
            .fetch_all(&mut *transaction)
            .await?;
        Ok(())
    }
}
