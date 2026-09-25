use super::*;

pub(super) async fn verify(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    wrong_release(context, cases).await;
    wrong_agent(context, cases).await;
}

async fn wrong_release(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    let admin_pool = context.admin_pool;
    let service = context.service;
    let fixture = context.fixture;
    let release_id = context.release_id;
    let release_agent_id = context.release_agent_id;
    let gateway_id = context.gateway_id;
    let revision_id = context.revision_id;
    let wrong_agent = seed_update_release(
        admin_pool,
        release_id,
        ReleaseAgentId::from_uuid(release_agent_id),
    )
    .await;
    let wrong_release: Uuid =
        sqlx::query_scalar("SELECT release_id FROM release_agents WHERE id = $1")
            .bind(wrong_agent.as_uuid())
            .fetch_one(admin_pool)
            .await
            .expect("read wrong release");
    let target = seed_same_org_target_project(admin_pool, fixture, "wrong-release").await;
    let wrong_revision = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id,
          release_agent_id, release_agent_key, handler_contract, exposure,
          parameters, secret_slots, mailbox_slots, service_loopback_port,
          service_readiness_path, service_health_path, service_log_capture_mode,
          normalized_hash, created_by)
         SELECT $1, gateway_id, project_id, repository_id, $2, $3,
                release_agent_key, handler_contract, exposure, parameters,
                secret_slots, mailbox_slots, service_loopback_port,
                service_readiness_path, service_health_path, service_log_capture_mode,
                $5, created_by
         FROM gateway_revisions WHERE id = $4",
    )
    .bind(wrong_revision)
    .bind(wrong_release)
    .bind(wrong_agent.as_uuid())
    .bind(revision_id)
    .bind(vec![8_u8; 32])
    .execute(admin_pool)
    .await
    .expect("seed wrong release revision");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         SELECT $1, $2, gateway_id, project_id, path, methods
         FROM gateway_routes WHERE gateway_revision_id = $3",
    )
    .bind(Uuid::new_v4())
    .bind(wrong_revision)
    .bind(revision_id)
    .execute(admin_pool)
    .await
    .expect("seed wrong release route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(wrong_revision)
        .execute(admin_pool)
        .await
        .expect("activate wrong release revision");
    assert_installation_denied_without_receipt(
        service,
        admin_pool,
        fixture.actor,
        target,
        release_id,
        cases.next("wrong-release"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(admin_pool)
        .await
        .expect("restore active release revision");
}

async fn wrong_agent(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    let admin_pool = context.admin_pool;
    let service = context.service;
    let fixture = context.fixture;
    let release_id = context.release_id;
    let release_agent_id = context.release_agent_id;
    let gateway_id = context.gateway_id;
    let revision_id = context.revision_id;
    let target = seed_same_org_target_project(admin_pool, fixture, "wrong-agent").await;
    let same_release_wrong_agent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state, update_hook)
         SELECT $1, release_id, family_id, agent_key || '-wrong', display_name,
                runtime_contract, $2, parameter_schema, secret_slot_schema,
                requires_state, update_hook
         FROM release_agents WHERE id = $3",
    )
    .bind(same_release_wrong_agent)
    .bind(vec![9_u8; 32])
    .bind(release_agent_id)
    .execute(admin_pool)
    .await
    .expect("seed same-release wrong agent");
    let wrong_agent_revision = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id,
          release_agent_id, release_agent_key, handler_contract, exposure,
          parameters, secret_slots, mailbox_slots, service_loopback_port,
          service_readiness_path, service_health_path, service_log_capture_mode,
          normalized_hash, created_by)
         SELECT $1, gateway_id, project_id, repository_id, release_id, $2,
                release_agent_key, handler_contract, exposure, parameters,
                secret_slots, mailbox_slots, service_loopback_port,
                service_readiness_path, service_health_path, service_log_capture_mode,
                $3, created_by
         FROM gateway_revisions WHERE id = $4",
    )
    .bind(wrong_agent_revision)
    .bind(same_release_wrong_agent)
    .bind(vec![10_u8; 32])
    .bind(revision_id)
    .execute(admin_pool)
    .await
    .expect("seed wrong agent revision");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         SELECT $1, $2, gateway_id, project_id, path, methods
         FROM gateway_routes WHERE gateway_revision_id = $3",
    )
    .bind(Uuid::new_v4())
    .bind(wrong_agent_revision)
    .bind(revision_id)
    .execute(admin_pool)
    .await
    .expect("seed wrong agent route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(wrong_agent_revision)
        .execute(admin_pool)
        .await
        .expect("activate wrong agent revision");
    assert_installation_denied_without_receipt(
        service,
        admin_pool,
        fixture.actor,
        target,
        release_id,
        cases.next("wrong-agent"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(admin_pool)
        .await
        .expect("restore active agent revision");
}
