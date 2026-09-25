use super::*;

pub(super) async fn seed_fixture(pool: &sqlx::PgPool, contract: &str) -> Fixture {
    seed_fixture_with_lease(pool, contract, false).await
}

pub(super) async fn seed_application_log_fixture(pool: &sqlx::PgPool, contract: &str) -> Fixture {
    seed_fixture_with_capture_mode(pool, contract, false, "application").await
}

#[allow(clippy::too_many_lines)]
pub(super) async fn seed_fixture_with_lease(
    pool: &sqlx::PgPool,
    contract: &str,
    expired_instance: bool,
) -> Fixture {
    seed_fixture_with_capture_mode(pool, contract, expired_instance, "disabled").await
}

#[allow(clippy::too_many_lines)]
pub(super) async fn seed_fixture_with_capture_mode(
    pool: &sqlx::PgPool,
    contract: &str,
    expired_instance: bool,
    capture_mode: &str,
) -> Fixture {
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
    let release = Uuid::new_v4();
    let build_request = Uuid::new_v4();
    let agent_family = Uuid::new_v4();
    let release_agent = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'recovery owner')")
        .bind(owner)
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("recovery-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("recovery-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("recovery-{repository}"))
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
    .bind(format!("recovery-{gateway}"))
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway");
    let service = contract == "http.service.v1";
    if service {
        let source_commit = format!("{:040x}", release.as_u128());
        sqlx::query(
            "INSERT INTO build_requests
                (id, repository_id, source_commit, source_ref,
                 build_definition_hash, state, created_by)
             VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
        )
        .bind(build_request)
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
                 configuration_hash, manifest_hash, state,
                 publication_actor_id, published_at)
             VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}',
                     $7, $8, 'published', $9, now())",
        )
        .bind(release)
        .bind(repository)
        .bind(format!("recovery-{}", release.simple()))
        .bind(&source_commit)
        .bind(build_request)
        .bind([4_u8; 32].as_slice())
        .bind([5_u8; 32].as_slice())
        .bind([6_u8; 32].as_slice())
        .bind(owner)
        .execute(pool)
        .await
        .expect("published release");
        sqlx::query(
            "INSERT INTO agent_families (id, repository_id, agent_key)
             VALUES ($1, $2, $3)",
        )
        .bind(agent_family)
        .bind(repository)
        .bind(format!("recovery-agent-{}", release.simple()))
        .execute(pool)
        .await
        .expect("agent family");
        sqlx::query(
            "INSERT INTO release_agents
                (id, release_id, family_id, agent_key, display_name,
                 runtime_contract, runtime_contract_hash, parameter_schema,
                 secret_slot_schema, requires_state)
             VALUES ($1, $2, $3, 'recovery-service', 'Recovery service',
                     '{}', $4, '[]', '[]', false)",
        )
        .bind(release_agent)
        .bind(release)
        .bind(agent_family)
        .bind([7_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("release agent");
    }
    let secret_slots: Vec<&str> = if service { vec!["hook"] } else { Vec::new() };
    let service_loopback_port = service.then_some(18080_i32);
    let service_readiness_path = service.then_some("/ready");
    let service_health_path = service.then_some("/health");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, service_log_capture_mode, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5,
                 $6, $7, $8, 'public', '{}', $9, $10, $11, $12, $13, $14, $15)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(if service { Some(release) } else { None })
    .bind(if service { Some(release_agent) } else { None })
    .bind(if service {
        Some("recovery-service")
    } else {
        None
    })
    .bind(contract)
    .bind(&secret_slots)
    .bind(service_loopback_port)
    .bind(service_readiness_path)
    .bind(service_health_path)
    .bind(if service { capture_mode } else { "disabled" })
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
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, $5, ARRAY['GET'])",
    )
    .bind(route)
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(format!("/service-{}", contract.replace('.', "-")))
    .execute(pool)
    .await
    .expect("route");
    let service_instance = if service {
        let instance = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, 'recovery-host', $4, 1,
                     $5, 'ready',
                     CASE WHEN $6 THEN now() - interval '10 minutes'
                          ELSE now() + interval '10 minutes' END,
                     CASE WHEN $6 THEN now() - interval '20 minutes'
                          ELSE now() END)",
        )
        .bind(instance)
        .bind(gateway)
        .bind(revision)
        .bind(owner)
        .bind(format!("gateway-service-{instance}"))
        .bind(expired_instance)
        .execute(pool)
        .await
        .expect("ready service instance");
        Some(instance)
    } else {
        None
    };
    Fixture {
        owner,
        organization,
        project,
        gateway,
        revision,
        route,
        service_instance,
    }
}
