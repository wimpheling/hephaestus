use identity_domain::UserId;
use uuid::Uuid;

pub async fn seed_secret_roles(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    repository_id: Uuid,
    user_id: UserId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO project_secret_roles (project_id, user_id, role)
           VALUES ($1, $2, 'secret_manager')
           ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(user_id.as_uuid())
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO repository_secret_roles (repository_id, user_id, role)
           VALUES ($1, $2, 'secret_manager')
           ON CONFLICT DO NOTHING",
    )
    .bind(repository_id)
    .bind(user_id.as_uuid())
    .execute(pool)
    .await?;
    Ok(())
}
