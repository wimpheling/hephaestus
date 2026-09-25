struct AppStore {
    app_pool: sqlx::PgPool,
}

impl AppStore {
    async fn resolve(&self) -> Result<(), sqlx::Error> {
        sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT generation_id FROM public.resolve_active_ui_generation_host($1)",
        )
        .fetch_optional(&self.app_pool)
        .await?;
        Ok(())
    }
}
