struct AppStore {
    app_pool: sqlx::PgPool,
}

impl AppStore {
    async fn resolve_then_read(&self) -> Result<(), sqlx::Error> {
        let mut transaction = self.app_pool.begin().await?;
        sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT generation_id FROM public.resolve_active_ui_generation_host($1)",
        )
        .fetch_optional(&mut *transaction)
        .await?;
        sqlx::query("SELECT id FROM projects")
            .fetch_all(&mut *transaction)
            .await?;
        Ok(())
    }
}
