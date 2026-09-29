use super::*;

pub async fn seed_fixture(pool: &sqlx::PgPool, contract: &str) -> Fixture {
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'ownership owner')")
        .bind(owner)
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("ownership-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("ownership-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("ownership-{repository}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query(
        "INSERT INTO gateways
            (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, $4, 'enabled', $5)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(format!("ownership-{}", &gateway.to_string()[..8]))
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway");
    let service = contract == "http.service.v1";
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, 'public', '{}', $6, $7, $8, $9, $10, $11)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(contract)
    .bind(if service {
        vec![String::from("hook")]
    } else {
        Vec::new()
    })
    .bind(service.then_some(18_080_i32))
    .bind(service.then_some("/ready"))
    .bind(service.then_some("/health"))
    .bind([9_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("revision");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway)
        .bind(revision)
        .execute(pool)
        .await
        .expect("active revision");
    Fixture { gateway, revision }
}

pub async fn seed_service_revision(pool: &sqlx::PgPool, gateway: Uuid) -> Uuid {
    let revision = Uuid::new_v4();
    let (repository, owner): (Uuid, Uuid) =
        sqlx::query_as("SELECT repository_id, created_by FROM gateways WHERE id = $1")
            .bind(gateway)
            .fetch_one(pool)
            .await
            .expect("gateway coordinates");
    let release = insert_published_release(pool, repository, owner).await;
    let mut revision_hash = [8_u8; 32];
    revision_hash[..16].copy_from_slice(revision.as_bytes());
    let result = sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port,
             service_readiness_path, service_health_path, normalized_hash,
             created_by)
         SELECT $2, gateway.id, gateway.project_id, gateway.repository_id, $3,
                (SELECT id FROM release_agents WHERE release_id = $3 LIMIT 1),
                'ownership-service', 'http.service.v1', 'public', '{}',
                '{hook}', 18081, '/ready', '/health', $4, gateway.created_by
           FROM gateways AS gateway
          WHERE gateway.id = $1",
    )
    .bind(gateway)
    .bind(revision)
    .bind(release)
    .bind(revision_hash.as_slice())
    .execute(pool)
    .await
    .expect("replacement service revision");
    assert_eq!(result.rows_affected(), 1);
    revision
}

pub async fn gateway_event_count(pool: &sqlx::PgPool, gateway: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(gateway)
    .fetch_one(pool)
    .await
    .expect("gateway event count")
}

pub async fn active_pointer(pool: &sqlx::PgPool, gateway: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
        .bind(gateway)
        .fetch_one(pool)
        .await
        .expect("active gateway pointer")
}

pub async fn insert_published_release(pool: &sqlx::PgPool, repository: Uuid, owner: Uuid) -> Uuid {
    let release = Uuid::new_v4();
    let build = Uuid::new_v4();
    let source_commit = format!("00000000{}", release.simple());
    sqlx::query(
        "INSERT INTO build_requests
            (id, repository_id, source_commit, source_ref,
             build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build)
    .bind(repository)
    .bind(&source_commit)
    .bind([4_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("build request");
    sqlx::query(
        "INSERT INTO releases
            (id, repository_id, version, source_commit, source_ref,
             build_request_id, build_definition_hash, configuration,
             configuration_hash, manifest_hash, state, publication_actor_id,
             published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8,
                 'published', $9, now())",
    )
    .bind(release)
    .bind(repository)
    .bind(format!("ownership-release-{release}"))
    .bind(&source_commit)
    .bind(build)
    .bind([4_u8; 32].as_slice())
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("published release");
    let family = Uuid::new_v4();
    let agent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(family)
    .bind(repository)
    .bind(format!("ownership-agent-{release}"))
    .execute(pool)
    .await
    .expect("agent family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name,
             runtime_contract, runtime_contract_hash, parameter_schema,
             secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'ownership-service', 'Ownership service', '{}',
                 $4, '[]', '[]', false)",
    )
    .bind(agent)
    .bind(release)
    .bind(family)
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("release agent");
    release
}
