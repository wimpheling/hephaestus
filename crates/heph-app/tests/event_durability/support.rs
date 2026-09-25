pub async fn set_actor(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: uuid::Uuid,
    request: uuid::Uuid,
) {
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                  set_config('hephaestus.subject_type', 'user', true),
                  set_config('hephaestus.request_id', $2, true),
                  set_config('hephaestus.occurrence_id', $2, true)",
    )
    .bind(actor.to_string())
    .bind(request.to_string())
    .execute(&mut **transaction)
    .await
    .expect("set event actor");
}

pub async fn update_project(
    pool: &sqlx::PgPool,
    actor: uuid::Uuid,
    project: uuid::Uuid,
    request: uuid::Uuid,
    key: &str,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    set_actor(&mut transaction, actor, request).await;
    sqlx::query(
        "UPDATE projects
           SET settings = settings || jsonb_build_object($2::text, true)
           WHERE id = $1",
    )
    .bind(project)
    .bind(key)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await
}

pub async fn transition_gateway(
    pool: &sqlx::PgPool,
    actor: uuid::Uuid,
    gateway: uuid::Uuid,
    request: uuid::Uuid,
) -> Result<bool, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    set_actor(&mut transaction, actor, request).await;
    let changed: bool =
        sqlx::query_scalar("SELECT gateway_transition_lifecycle($1, 'enabled', 'paused', $2, $3)")
            .bind(gateway)
            .bind(actor)
            .bind(request)
            .fetch_one(&mut *transaction)
            .await?;
    transaction.commit().await?;
    Ok(changed)
}
