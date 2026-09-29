use sqlx::PgPool;
use uuid::Uuid;

use super::fixture::Fixture;
use super::{assert_sqlstate, seed_repository_installation};

pub async fn assert_active_owner_key_is_unique(pool: &PgPool, fixture: &Fixture) {
    let duplicate_project = sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, NULL, 'project', $3, 'enabled', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.project)
    .bind(fixture.ui_key)
    .bind(fixture.generation)
    .bind(fixture.actor)
    .execute(pool)
    .await;
    assert_sqlstate(duplicate_project, "23505");

    let duplicate_repository = sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, 'repository', $4, 'enabled', $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.project)
    .bind(fixture.repository)
    .bind(fixture.repository_ui_key)
    .bind(fixture.repository_generation)
    .bind(fixture.actor)
    .execute(pool)
    .await;
    assert_sqlstate(duplicate_repository, "23505");

    sqlx::query(
        "UPDATE ui_installations
         SET lifecycle = 'removed', removed_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(fixture.installation)
    .execute(pool)
    .await
    .expect("remove original project installation");
    let replacement_installation = Uuid::new_v4();
    let replacement_generation = Uuid::new_v4();
    seed_project_installation(
        pool,
        fixture,
        replacement_installation,
        replacement_generation,
    )
    .await;

    sqlx::query(
        "UPDATE ui_installations
         SET lifecycle = 'removed', removed_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(fixture.repository_installation)
    .execute(pool)
    .await
    .expect("remove original repository installation");
    let repository_replacement_installation = Uuid::new_v4();
    let repository_replacement_generation = Uuid::new_v4();
    seed_repository_installation(
        pool,
        fixture,
        repository_replacement_installation,
        repository_replacement_generation,
        fixture.repository,
    )
    .await;
}

pub async fn seed_project_installation(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
) {
    let mut tx = pool.begin().await.expect("begin project replacement");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, NULL, 'project', $3, 'enabled', $4, $5)",
    )
    .bind(installation)
    .bind(fixture.project)
    .bind(fixture.ui_key)
    .bind(generation)
    .bind(fixture.actor)
    .execute(&mut *tx)
    .await
    .expect("reuse removed project owner key");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'project')",
    )
    .bind(generation)
    .bind(installation)
    .bind(fixture.release)
    .bind(fixture.ui_key)
    .execute(&mut *tx)
    .await
    .expect("replacement project generation");
    tx.commit().await.expect("commit project replacement");
}
