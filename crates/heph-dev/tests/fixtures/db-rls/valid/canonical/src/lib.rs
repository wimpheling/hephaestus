use authz_postgres::begin_actor_transaction;

struct AppStore {
    app_pool: sqlx::PgPool,
}

impl AppStore {
    async fn read(&self, identity: &Identity) -> Result<(), sqlx::Error> {
        let mut transaction = begin_actor_transaction(&self.app_pool, identity).await?;
        sqlx::query("SELECT id FROM projects WHERE id = $1")
            .fetch_one(&mut *transaction)
            .await?;
        Ok(())
    }
}

struct Identity;
