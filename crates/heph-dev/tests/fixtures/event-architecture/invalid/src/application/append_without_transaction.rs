async fn append_later(connection: &sqlx::PgPool) {
    sqlx::query("SELECT append_application_event($1, 'project', $2, 'project', $2, 'project.changed', 'updated', 'active', NULL, NULL)")
        .execute(connection)
        .await
        .unwrap();
}
