use super::{model::digest, seed::SeedIds};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn seed_installations(worker: &PgPool, ids: &SeedIds) {
    insert_canonical_session(worker, ids.parent_session, ids.actor, ids.request_id, 20).await;
    insert_canonical_session(
        worker,
        ids.outsider_parent_session,
        ids.outsider,
        Uuid::new_v4(),
        20,
    )
    .await;
    seed_project_installation(
        worker,
        ids.installation,
        ids.generation,
        ids.release,
        "schema-ui",
        ids.actor,
        ids.project,
    )
    .await;
    seed_global_installation(
        worker,
        ids.global_installation,
        ids.global_generation,
        ids.release,
        "schema-global",
        ids.actor,
        ids.organization,
    )
    .await;
    seed_repository_installation(
        worker,
        ids.repository_installation,
        ids.repository_generation,
        ids.release,
        "schema-repository",
        ids.actor,
        ids.source_project,
        ids.repository,
    )
    .await;
    seed_project_installation(
        worker,
        ids.other_installation,
        ids.other_generation,
        ids.release,
        "schema-ui-two",
        ids.actor,
        ids.project,
    )
    .await;
    seed_project_installation(
        worker,
        ids.managed_installation,
        ids.managed_generation,
        ids.release,
        "schema-managed",
        ids.actor,
        ids.project,
    )
    .await;
    sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key,
          release_id, ui_key, gateway_id, gateway_revision_id,
          release_agent_id, gateway_name, method, route, exposure)
         VALUES
            ($1, $2, 'api', 'status', $3, 'schema-ui', $4, $5, $6,
             'browser-service', 'POST', '/service/api', 'heph_authenticated'),
            ($7, $8, 'managed_service', 'service', $3, 'schema-managed',
             $4, $5, $6, 'browser-service', 'GET', '/service',
             'heph_authenticated'),
            ($7, $8, 'api', 'status', $3, 'schema-managed', $4, $5, $6,
             'browser-service', 'POST', '/service/api', 'heph_authenticated')",
    )
    .bind(ids.installation)
    .bind(ids.generation)
    .bind(ids.release)
    .bind(ids.managed_gateway)
    .bind(ids.managed_revision)
    .bind(ids.release_agent)
    .bind(ids.managed_installation)
    .bind(ids.managed_generation)
    .execute(worker)
    .await
    .expect("seed static and managed/API installation bindings");
}

async fn insert_canonical_session(
    worker: &PgPool,
    session_id: Uuid,
    user_id: Uuid,
    request_id: Uuid,
    expiry_hours: i64,
) {
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + ($7::int * interval '1 hour'))",
    )
    .bind(session_id)
    .bind(digest(200))
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(digest(201))
    .bind(user_id)
    .bind(expiry_hours)
    .execute(worker)
    .await
    .expect("seed canonical human browser session");
}

async fn seed_project_installation(
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

async fn seed_global_installation(
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

// The fixture helper keeps the installation identity tuple explicit at each
// call site so tests cannot accidentally mix repository and project scope.
#[allow(clippy::too_many_arguments)]
async fn seed_repository_installation(
    worker: &PgPool,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
    actor: Uuid,
    project: Uuid,
    repository: Uuid,
) {
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
         VALUES ($1, $2, 1, $3, $4, 'repository', 'read_write')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed repository UI generation");
    tx.commit()
        .await
        .expect("commit repository UI installation");
}
