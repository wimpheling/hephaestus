async fn set_verified_actor_context(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true),
                set_config('hephaestus.request_id', $2, true)",
    )
    .execute(&mut **transaction)
    .await
    .map(|_| ())
}
