mod nested {
    async fn direct_write(transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>) {
        sqlx::query("INSERT INTO application_events (id) VALUES ($1)")
            .execute(&mut **transaction)
            .await
            .unwrap();
    }
}
