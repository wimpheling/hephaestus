use super::{installations, model::Fixture, ui_graph};
use sqlx::PgPool;
use uuid::Uuid;

pub struct SeedIds {
    pub actor: Uuid,
    pub outsider: Uuid,
    pub organization: Uuid,
    pub other_organization: Uuid,
    pub project: Uuid,
    pub source_project: Uuid,
    pub repository: Uuid,
    pub receive: Uuid,
    pub build: Uuid,
    pub source_revision: Uuid,
    pub release: Uuid,
    pub artifact: Uuid,
    pub parent_session: Uuid,
    pub outsider_parent_session: Uuid,
    pub installation: Uuid,
    pub other_installation: Uuid,
    pub generation: Uuid,
    pub other_generation: Uuid,
    pub global_installation: Uuid,
    pub global_generation: Uuid,
    pub repository_installation: Uuid,
    pub repository_generation: Uuid,
    pub managed_installation: Uuid,
    pub managed_generation: Uuid,
    pub managed_gateway: Uuid,
    pub managed_revision: Uuid,
    pub release_agent: Uuid,
    pub agent_family: Uuid,
    pub request_id: Uuid,
}

impl SeedIds {
    fn new() -> Self {
        Self {
            actor: Uuid::new_v4(),
            outsider: Uuid::new_v4(),
            organization: Uuid::new_v4(),
            other_organization: Uuid::new_v4(),
            project: Uuid::new_v4(),
            source_project: Uuid::new_v4(),
            repository: Uuid::new_v4(),
            receive: Uuid::new_v4(),
            build: Uuid::new_v4(),
            source_revision: Uuid::new_v4(),
            release: Uuid::new_v4(),
            artifact: Uuid::new_v4(),
            parent_session: Uuid::new_v4(),
            outsider_parent_session: Uuid::new_v4(),
            installation: Uuid::new_v4(),
            other_installation: Uuid::new_v4(),
            generation: Uuid::new_v4(),
            other_generation: Uuid::new_v4(),
            global_installation: Uuid::new_v4(),
            global_generation: Uuid::new_v4(),
            repository_installation: Uuid::new_v4(),
            repository_generation: Uuid::new_v4(),
            managed_installation: Uuid::new_v4(),
            managed_generation: Uuid::new_v4(),
            managed_gateway: Uuid::new_v4(),
            managed_revision: Uuid::new_v4(),
            release_agent: Uuid::new_v4(),
            agent_family: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
        }
    }
}

pub async fn seed_fixture_reusing_installation_helpers(worker: &PgPool) -> Fixture {
    seed_fixture_reusing_installation_helpers_with_publication(worker, true).await
}

pub async fn seed_fixture_reusing_installation_helpers_draft(worker: &PgPool) -> Fixture {
    seed_fixture_reusing_installation_helpers_with_publication(worker, false).await
}

async fn seed_fixture_reusing_installation_helpers_with_publication(
    worker: &PgPool,
    publish_release: bool,
) -> Fixture {
    let ids = SeedIds::new();
    seed_core(worker, &ids).await;
    ui_graph::seed_ui_graph(worker, &ids, publish_release).await;
    installations::seed_installations(worker, &ids).await;
    Fixture {
        actor: ids.actor,
        outsider: ids.outsider,
        organization: ids.organization,
        project: ids.project,
        source_project: ids.source_project,
        repository: ids.repository,
        release: ids.release,
        release_agent: ids.release_agent,
        other_organization: ids.other_organization,
        parent_session: ids.parent_session,
        outsider_parent_session: ids.outsider_parent_session,
        installation: ids.installation,
        other_installation: ids.other_installation,
        generation: ids.generation,
        other_generation: ids.other_generation,
        global_installation: ids.global_installation,
        global_generation: ids.global_generation,
        repository_installation: ids.repository_installation,
        repository_generation: ids.repository_generation,
        managed_installation: ids.managed_installation,
        managed_generation: ids.managed_generation,
        managed_gateway: ids.managed_gateway,
        managed_revision: ids.managed_revision,
        route: "schema-ui",
    }
}

// Core release/repository rows are kept together so all downstream UI rows
// reference the same durable graph.
#[allow(clippy::too_many_lines)]
async fn seed_core(worker: &PgPool, ids: &SeedIds) {
    let actor = ids.actor;
    let outsider = ids.outsider;
    let organization = ids.organization;
    let other_organization = ids.other_organization;
    let project = ids.project;
    let source_project = ids.source_project;
    let repository = ids.repository;
    let receive = ids.receive;
    let build = ids.build;
    let source_revision = ids.source_revision;
    let release = ids.release;
    let artifact = ids.artifact;
    let release_agent = ids.release_agent;
    let agent_family = ids.agent_family;
    sqlx::query(
        "INSERT INTO users (id, display_name) VALUES ($1, 'UI browser actor'), ($2, 'UI browser outsider')",
    )
    .bind(actor)
    .bind(outsider)
    .execute(worker)
    .await
    .expect("seed users");
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(actor.to_string())
        .execute(worker)
        .await
        .expect("set fixture actor");
    sqlx::query(
        "INSERT INTO organizations (id, name) VALUES ($1, 'UI browser org'), ($2, 'Other org')",
    )
    .bind(organization)
    .bind(other_organization)
    .execute(worker)
    .await
    .expect("seed organizations");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed organization owner");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
         VALUES ($1, $2, 'ui-browser-target'), ($3, $2, 'ui-browser-source')",
    )
    .bind(project)
    .bind(organization)
    .bind(source_project)
    .execute(worker)
    .await
    .expect("seed project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'ui-browser-repository')",
    )
    .bind(repository)
    .bind(source_project)
    .execute(worker)
    .await
    .expect("seed repository");
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'ui-browser-schema', 'accepted', now())",
    )
    .bind(receive)
    .bind(repository)
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed accepted receive");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'succeeded', $6)",
    )
    .bind(build)
    .bind(repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(receive)
    .bind(vec![1_u8; 32])
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed build request");
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config, normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', '{}'::jsonb, $7)",
    )
    .bind(source_revision)
    .bind(repository)
    .bind(receive)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    .bind(vec![2_u8; 32])
    .bind(vec![3_u8; 32])
    .execute(worker)
    .await
    .expect("seed source manifest revision");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit, source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(build)
    .bind(repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(source_revision)
    .execute(worker)
    .await
    .expect("link source manifest revision");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref, build_request_id,
          build_definition_hash, configuration, configuration_hash, manifest_hash, state)
         VALUES ($1, $2, 'ui-browser-v1', $3, 'refs/heads/main', $4, $5,
                 '{}'::jsonb, $6, $7, 'draft')",
    )
    .bind(release)
    .bind(repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(build)
    .bind(vec![1_u8; 32])
    .bind(vec![2_u8; 32])
    .bind(vec![3_u8; 32])
    .execute(worker)
    .await
    .expect("seed published release");
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(release)
    .bind(build)
    .bind(source_revision)
    .execute(worker)
    .await
    .expect("seed release UI snapshot");
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes,
          media_type, storage_key)
         VALUES ($1, $2, 'dist/index.html', 'file', 420, $3, 12,
                 'text/html', $4)",
    )
    .bind(artifact)
    .bind(release)
    .bind([7_u8; 32].as_slice())
    .bind(Uuid::new_v4())
    .execute(worker)
    .await
    .expect("seed static UI artifact");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'browser-service')",
    )
    .bind(agent_family)
    .bind(repository)
    .execute(worker)
    .await
    .expect("seed browser service family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'browser-service', 'Browser service',
                 '{}'::jsonb, $4, '[]'::jsonb, '[]'::jsonb, false)",
    )
    .bind(release_agent)
    .bind(release)
    .bind(agent_family)
    .bind(vec![4_u8; 32])
    .execute(worker)
    .await
    .expect("seed browser service release agent");
}
