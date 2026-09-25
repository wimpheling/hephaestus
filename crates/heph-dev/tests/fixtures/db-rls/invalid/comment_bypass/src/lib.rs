struct AppStore {
    app_pool: sqlx::PgPool,
}

impl AppStore {
    async fn delete(&self) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM projects /* resolve_active_ui_generation_host($1) */")
            .execute(&self.app_pool)
            .await?;
        Ok(())
    }
}
