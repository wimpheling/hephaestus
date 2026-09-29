use super::*;

pub async fn seed_project_installation(
    worker: &PgPool,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
    actor: Uuid,
    project: Uuid,
) {
    let mut tx = worker.begin().await.expect("begin installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, NULL, 'project', $3, 'enabled', $4, $5)",
    )
    .bind(installation_id)
    .bind(project)
    .bind(ui_key)
    .bind(generation_id)
    .bind(actor)
    .execute(&mut *tx)
    .await
    .expect("seed project UI installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'project')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed project UI generation");
    tx.commit().await.expect("commit project UI installation");
}

pub async fn seed_global_installation(
    worker: &PgPool,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
    actor: Uuid,
    organization: Uuid,
) {
    let mut tx = worker
        .begin()
        .await
        .expect("begin global installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', $3, 'enabled', $4, $5)",
    )
    .bind(installation_id)
    .bind(organization)
    .bind(ui_key)
    .bind(generation_id)
    .bind(actor)
    .execute(&mut *tx)
    .await
    .expect("seed global UI installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'global')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed global UI generation");
    tx.commit().await.expect("commit global UI installation");
}

// Keep every repository owner and generation column explicit in this fixture
// so the scope and composite-FK proof stays readable at the SQL boundary.
#[allow(clippy::too_many_arguments)]
pub async fn seed_repository_installation(
    worker: &PgPool,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
    actor: Uuid,
    project: Uuid,
    repository: Uuid,
) {
    let repository_git_access = match ui_key {
        "schema-repository" => "read",
        "schema-repository-write" => "read_write",
        _ => "none",
    };
    let mut tx = worker
        .begin()
        .await
        .expect("begin repository installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, 'repository', $4, 'enabled', $5, $6)",
    )
    .bind(installation_id)
    .bind(project)
    .bind(repository)
    .bind(ui_key)
    .bind(generation_id)
    .bind(actor)
    .execute(&mut *tx)
    .await
    .expect("seed repository UI installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope,
          repository_git_access)
         VALUES ($1, $2, 1, $3, $4, 'repository', $5)",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .bind(repository_git_access)
    .execute(&mut *tx)
    .await
    .expect("seed repository UI generation");
    tx.commit()
        .await
        .expect("commit repository UI installation");
}
