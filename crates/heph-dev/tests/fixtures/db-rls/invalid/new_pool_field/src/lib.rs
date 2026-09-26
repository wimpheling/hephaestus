struct AppStore {
    app_pool: sqlx::PgPool,
    newly_added_app_pool: sqlx::PgPool,
}
