use super::*;

#[path = "rejection_matrix/authorization.rs"]
mod authorization;
#[path = "rejection_matrix/gateway_variants.rs"]
mod gateway_variants;

struct CaseCounter(u32);

impl CaseCounter {
    fn next(&mut self, label: &'static str) -> String {
        self.0 += 1;
        format!("general-negative-{}-{label}", self.0)
    }
}

struct MatrixContext<'a> {
    admin_pool: &'a PgPool,
    service: &'a ReleaseService,
    fixture: &'a Fixture,
    release_id: ReleaseId,
    release_agent_id: Uuid,
    gateway_id: Uuid,
    revision_id: Uuid,
    source_repository: Uuid,
    source_project: Uuid,
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn install_ui_rejects_gateway_and_authority_mutations_without_receipt() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool().await else {
        return;
    };
    let fixture = seed(&admin_pool).await;
    let release_id = publish_managed_release(&admin_pool, &worker_pool, &fixture).await;
    let (release_agent_id, release_agent_key): (Uuid, String) =
        sqlx::query_as("SELECT id, agent_key FROM release_agents WHERE release_id = $1")
            .bind(release_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("published release agent");
    let (gateway_id, revision_id) = seed_active_ui_gateway(
        &admin_pool,
        &fixture,
        release_id,
        release_agent_id,
        &release_agent_key,
        "http.service.v1",
        "heph_authenticated",
        "/service",
        &["GET"],
    )
    .await;
    let service = ReleaseService::new(worker_pool, Arc::new(PostgresMelangeAuthorizer));
    let (source_repository, source_project): (Uuid, Uuid) = sqlx::query_as(
        "SELECT repository.id, repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(release_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("read source project");

    let mut cases = CaseCounter(0);

    let context = MatrixContext {
        admin_pool: &admin_pool,
        service: &service,
        fixture: &fixture,
        release_id,
        release_agent_id,
        gateway_id,
        revision_id,
        source_repository,
        source_project,
    };
    gateway_variants::verify(&context, &mut cases).await;
    authorization::verify(&context, &mut cases).await;
    println!("REAL_GENERAL_INSTALL_NEGATIVES=1 cases={}", cases.0);
}
