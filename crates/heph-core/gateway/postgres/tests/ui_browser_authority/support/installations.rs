//! Installation, session, and handoff fixture rows.

use super::children::digest;
use super::fixture::SeedIds;
use sqlx::PgPool;
use uuid::Uuid;

#[allow(clippy::too_many_lines)]
pub async fn seed_installations(worker: &PgPool, ids: &SeedIds) {
    let SeedIds {
        actor,
        outsider,
        organization,
        project,
        source_project,
        repository,
        release,
        parent_session,
        outsider_parent_session,
        installation,
        other_installation,
        generation,
        other_generation,
        global_installation,
        global_generation,
        repository_installation,
        repository_generation,
        managed_installation,
        managed_generation,
        managed_gateway,
        managed_revision,
        release_agent,
        request_id,
        ..
    } = *ids;

    insert_canonical_session(worker, parent_session, actor, request_id, 20).await;
    insert_canonical_session(
        worker,
        outsider_parent_session,
        outsider,
        Uuid::new_v4(),
        20,
    )
    .await;
    seed_project_installation(
        worker,
        installation,
        generation,
        release,
        "schema-ui",
        actor,
        project,
    )
    .await;
    seed_global_installation(
        worker,
        global_installation,
        global_generation,
        release,
        "schema-global",
        actor,
        organization,
    )
    .await;
    seed_repository_installation(
        worker,
        repository_installation,
        repository_generation,
        release,
        "schema-repository",
        actor,
        source_project,
        repository,
    )
    .await;
    seed_project_installation(
        worker,
        other_installation,
        other_generation,
        release,
        "schema-ui-two",
        actor,
        project,
    )
    .await;
    seed_project_installation(
        worker,
        managed_installation,
        managed_generation,
        release,
        "schema-managed",
        actor,
        project,
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
    .bind(installation)
    .bind(generation)
    .bind(release)
    .bind(managed_gateway)
    .bind(managed_revision)
    .bind(release_agent)
    .bind(managed_installation)
    .bind(managed_generation)
    .execute(worker)
    .await
    .expect("seed static and managed/API installation bindings");
}

pub async fn insert_canonical_session(
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

// Keep the explicit installation fixture arguments aligned with the schema
// rows it creates; grouping them would hide the target-scope identity.
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
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'repository')",
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
