//! Shared gateway mailbox fixture for `PostgreSQL` integration tests.

use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct Fixture {
    pub owner: Uuid,
    pub project: Uuid,
    pub repository: Uuid,
    pub family: Uuid,
    pub gateway: Uuid,
    pub revision: Uuid,
    pub mailbox: Uuid,
    pub grant: Uuid,
    pub invocation: Uuid,
    pub session: Uuid,
    pub release: Uuid,
}

#[allow(clippy::too_many_lines)]
pub async fn seed_fixture(pool: &sqlx::PgPool) -> Fixture {
    let owner_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let repository_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let build_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let instance_revision_id = Uuid::new_v4();
    let mailbox_id = Uuid::new_v4();
    let gateway_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let route_id = Uuid::new_v4();
    let binding_id = Uuid::new_v4();
    let grant_id = Uuid::new_v4();
    let invocation_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let hash = [9_u8; 32];

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Gateway mailbox test owner')")
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("gateway-mailbox-{organization_id}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(organization_id)
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("gateway-mailbox-{project_id}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project_id)
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("project maintainer");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository_id)
        .bind(project_id)
        .bind(format!("gateway-mailbox-{repository_id}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query("INSERT INTO agent_families (id, repository_id, agent_key) VALUES ($1, $2, 'gateway-mailbox')")
        .bind(family_id)
        .bind(repository_id)
        .execute(pool)
        .await
        .expect("agent family");
    sqlx::query("INSERT INTO build_requests (id, repository_id, source_commit, source_ref, build_definition_hash, state, completed_at) VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())")
        .bind(build_id)
        .bind(repository_id)
        .bind("a".repeat(40))
        .bind(hash.as_slice())
        .execute(pool)
        .await
        .expect("build request");
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state, published_at) VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'published', now())")
        .bind(release_id).bind(repository_id).bind(format!("gateway-mailbox-{release_id}"))
        .bind("a".repeat(40)).bind(build_id).bind(hash.as_slice()).bind(hash.as_slice()).bind(hash.as_slice())
        .execute(pool).await.expect("release");
    sqlx::query("INSERT INTO release_agents (id, release_id, family_id, agent_key, display_name, runtime_contract, runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state) VALUES ($1, $2, $3, 'gateway-mailbox', 'Gateway mailbox', '{}', $4, '[]', '[]', false)")
        .bind(release_agent_id).bind(release_id).bind(family_id).bind(hash.as_slice())
        .execute(pool).await.expect("release agent");
    sqlx::query("INSERT INTO agent_instances (id, project_id, family_id, name, state) VALUES ($1, $2, $3, $4, 'active')")
        .bind(instance_id).bind(project_id).bind(family_id).bind(format!("gateway-mailbox-{instance_id}"))
        .execute(pool).await.expect("agent instance");
    sqlx::query("INSERT INTO agent_instance_revisions (id, instance_id, release_agent_id, parameters, parameter_hash, resource_selection, network_restriction, effective_runtime_policy, effective_policy_hash, platform_policy_version, runnable) VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5, 'test/v1', true)")
        .bind(instance_revision_id).bind(instance_id).bind(release_agent_id).bind(hash.as_slice()).bind(hash.as_slice())
        .execute(pool).await.expect("agent revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(instance_revision_id)
        .execute(pool)
        .await
        .expect("activate instance");
    sqlx::query(
        "INSERT INTO mailboxes (id, project_id, instance_id, state) VALUES ($1, $2, $3, 'active')",
    )
    .bind(mailbox_id)
    .bind(project_id)
    .bind(instance_id)
    .execute(pool)
    .await
    .expect("mailbox");
    sqlx::query("INSERT INTO gateways (id, project_id, repository_id, name, lifecycle, created_by) VALUES ($1, $2, $3, $4, 'enabled', $5)")
        .bind(gateway_id).bind(project_id).bind(repository_id).bind(format!("gateway-{gateway_id}"))
        .bind(owner_id).execute(pool).await.expect("gateway");
    sqlx::query("INSERT INTO gateway_revisions (id, gateway_id, project_id, repository_id, release_id, release_agent_id, release_agent_key, handler_contract, exposure, parameters, mailbox_slots, normalized_hash, created_by) VALUES ($1, $2, $3, $4, $5, $6, 'gateway-mailbox', 'http.v1', 'public', '{}', ARRAY['deliver', 'other'], $7, $8)")
        .bind(revision_id).bind(gateway_id).bind(project_id).bind(repository_id).bind(release_id).bind(release_agent_id).bind(hash.as_slice()).bind(owner_id)
        .execute(pool).await.expect("gateway revision");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate gateway revision");
    sqlx::query(
        "INSERT INTO project_capability_granters (project_id, user_id, created_by)
         VALUES ($1, $2, $2)",
    )
    .bind(project_id)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("grant gateway mailbox delegation authority");
    sqlx::query("INSERT INTO gateway_routes (id, gateway_revision_id, gateway_id, project_id, path, methods) VALUES ($1, $2, $3, $4, '/telegram', ARRAY['POST'])")
        .bind(route_id).bind(revision_id).bind(gateway_id).bind(project_id).execute(pool).await.expect("gateway route");
    sqlx::query("INSERT INTO gateway_mailbox_bindings (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by) VALUES ($1, $2, $3, $4, 'deliver', $5, 'gateway-proof', $6)")
        .bind(binding_id).bind(revision_id).bind(gateway_id).bind(project_id).bind(mailbox_id).bind(owner_id)
        .execute(pool).await.expect("gateway mailbox binding");
    sqlx::query("INSERT INTO gateway_mailbox_binding_grants (id, binding_id, status, granted_by) VALUES ($1, $2, 'active', $3)")
        .bind(grant_id).bind(binding_id).bind(owner_id).execute(pool).await.expect("authorized gateway mailbox grant");
    insert_invocation_and_session(
        pool,
        invocation_id,
        session_id,
        snapshot_id,
        gateway_id,
        revision_id,
        route_id,
        project_id,
        mailbox_id,
        grant_id,
        binding_id,
    )
    .await;

    Fixture {
        owner: owner_id,
        project: project_id,
        repository: repository_id,
        family: family_id,
        gateway: gateway_id,
        revision: revision_id,
        mailbox: mailbox_id,
        grant: grant_id,
        invocation: invocation_id,
        session: session_id,
        release: release_id,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_invocation_and_session(
    pool: &sqlx::PgPool,
    invocation_id: Uuid,
    session_id: Uuid,
    snapshot_id: Uuid,
    gateway_id: Uuid,
    revision_id: Uuid,
    route_id: Uuid,
    project_id: Uuid,
    mailbox_id: Uuid,
    grant_id: Uuid,
    binding_id: Uuid,
) {
    let hash = [7_u8; 32];
    let credential_hash = Sha256::digest(session_id.as_bytes());
    sqlx::query("INSERT INTO gateway_invocations (id, gateway_id, gateway_revision_id, gateway_route_id, project_id, request_id, outcome) VALUES ($1, $2, $3, $4, $5, $6, 'accepted')")
        .bind(invocation_id).bind(gateway_id).bind(revision_id).bind(route_id).bind(project_id).bind(Uuid::new_v4())
        .execute(pool).await.expect("accepted invocation");
    sqlx::query("INSERT INTO gateway_authorization_snapshots (id, invocation_id, gateway_id, gateway_revision_id, authorization_model_version, normalized_hash) VALUES ($1, $2, $3, $4, 'test/v1', $5)")
        .bind(snapshot_id).bind(invocation_id).bind(gateway_id).bind(revision_id).bind(hash.as_slice())
        .execute(pool).await.expect("authorization snapshot");
    sqlx::query("INSERT INTO gateway_runtime_authority_sessions (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id, identity_hash, snapshot_hash, issuance_generation, credential_hash, status, issued_at, expires_at, acknowledged_at) VALUES ($1, $2, $3, $4, $5, $6, $6, 1, $7, 'active', now(), now() + interval '10 minutes', now())")
        .bind(session_id).bind(snapshot_id).bind(invocation_id).bind(gateway_id).bind(revision_id).bind(hash.as_slice()).bind(credential_hash.as_slice())
        .execute(pool).await.expect("active runtime session");
    sqlx::query(
        "INSERT INTO gateway_authorization_snapshot_bindings
             (snapshot_id, gateway_revision_id, ordinal, binding_id, grant_id, binding_hash,
              slot_key, resource_kind, resource_id, granted_operations)
         VALUES ($1, $2, 0, $3, $4, $5, 'deliver', 'mailbox', $6, ARRAY['publish'])",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(binding_id)
    .bind(grant_id)
    .bind(hash.as_slice())
    .bind(mailbox_id)
    .execute(pool)
    .await
    .expect("exact mailbox authority snapshot binding");
}

pub async fn revoke_grant(pool: &sqlx::PgPool, grant_id: Uuid, owner_id: Uuid) {
    sqlx::query("UPDATE gateway_mailbox_binding_grants SET status = 'revoked', revoked_at = now(), revoked_by = $2 WHERE id = $1")
        .bind(grant_id).bind(owner_id).execute(pool).await.expect("authorized grant revocation");
}
