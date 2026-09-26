use sqlx::PgPool;
use uuid::Uuid;

use super::fixture::Fixture;

pub async fn seed_installation_rows(pool: &PgPool, fixture: &Fixture) {
    let mut tx = pool.begin().await.expect("begin installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, 'project', $4, 'enabled', $5, $6)",
    )
    .bind(fixture.installation)
    .bind(fixture.project)
    .bind(Option::<Uuid>::None)
    .bind(fixture.ui_key)
    .bind(fixture.generation)
    .bind(fixture.actor)
    .execute(&mut *tx)
    .await
    .expect("seed installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'project')",
    )
    .bind(fixture.generation)
    .bind(fixture.installation)
    .bind(fixture.release)
    .bind(fixture.ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed installation generation");
    sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key, release_id,
          ui_key, gateway_id, gateway_revision_id, release_agent_id, gateway_name,
          method, route, exposure)
         VALUES ($1, $2, 'api', $3, $4, $5, $6, $7, $8, 'ui-gateway',
                 'GET', '/ui', 'heph_authenticated')",
    )
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.binding_key)
    .bind(fixture.release)
    .bind(fixture.ui_key)
    .bind(fixture.gateway)
    .bind(fixture.gateway_revision)
    .bind(fixture.release_agent)
    .execute(&mut *tx)
    .await
    .expect("seed installation binding");
    sqlx::query(
        "INSERT INTO ui_installation_commands
         (command_key, caller_idempotency_key, operation, installation_id, actor_id,
          request_id, input_hash, result_generation_id, result_lifecycle)
         VALUES ($1, 'schema-install', 'install', $2, $3, $4, $5, $6, 'enabled')",
    )
    .bind(fixture.command.as_slice())
    .bind(fixture.installation)
    .bind(fixture.actor)
    .bind(Uuid::new_v4())
    .bind(vec![7_u8; 32])
    .bind(fixture.generation)
    .execute(&mut *tx)
    .await
    .expect("seed installation command");
    tx.commit().await.expect("commit installation seed");
    seed_repository_installation(
        pool,
        fixture,
        fixture.repository_installation,
        fixture.repository_generation,
        fixture.repository,
    )
    .await;
    seed_repository_installation(
        pool,
        fixture,
        fixture.repository_two_installation,
        fixture.repository_two_generation,
        fixture.repository_two,
    )
    .await;
}

pub async fn seed_repository_installation(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
    repository: Uuid,
) {
    let mut tx = pool
        .begin()
        .await
        .expect("begin repository installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, 'repository', $4, 'enabled', $5, $6)",
    )
    .bind(installation)
    .bind(fixture.project)
    .bind(repository)
    .bind(fixture.repository_ui_key)
    .bind(generation)
    .bind(fixture.actor)
    .execute(&mut *tx)
    .await
    .expect("seed repository installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'repository')",
    )
    .bind(generation)
    .bind(installation)
    .bind(fixture.release)
    .bind(fixture.repository_ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed repository generation");
    tx.commit()
        .await
        .expect("commit repository installation seed");
}
