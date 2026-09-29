use identity_domain::UserId;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::support::Fixture;

// The fixture mirrors the cross-table authorization graph in one ordered transaction setup.
#[allow(clippy::too_many_lines)]
pub async fn seed(pool: &PgPool) -> Fixture {
    let fixture = Fixture {
        owner: UserId::new(),
        admin: UserId::new(),
        maintainer: UserId::new(),
        member: UserId::new(),
        outsider: UserId::new(),
        revoked: UserId::new(),
        organization: Uuid::new_v4(),
        project: Uuid::new_v4(),
        consuming_project: Uuid::new_v4(),
        private_repository: Uuid::new_v4(),
        public_repository: Uuid::new_v4(),
        consuming_repository: Uuid::new_v4(),
        build: Uuid::new_v4(),
        release: Uuid::new_v4(),
        artifact: Uuid::new_v4(),
        release_agent: Uuid::new_v4(),
        instance: Uuid::new_v4(),
        attachment: Uuid::new_v4(),
        update: Uuid::new_v4(),
        run: Uuid::new_v4(),
        volume: Uuid::new_v4(),
    };
    for (user, name) in [
        (fixture.owner, "owner"),
        (fixture.admin, "admin"),
        (fixture.maintainer, "maintainer"),
        (fixture.member, "member"),
        (fixture.outsider, "outsider"),
        (fixture.revoked, "revoked"),
    ] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(user.as_uuid())
            .bind(name)
            .execute(pool)
            .await
            .expect("seed user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'organization')")
        .bind(fixture.organization)
        .execute(pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner'), ($1, $3, 'admin'), ($1, $4, 'member')",
    )
    .bind(fixture.organization)
    .bind(fixture.owner.as_uuid())
    .bind(fixture.admin.as_uuid())
    .bind(fixture.member.as_uuid())
    .execute(pool)
    .await
    .expect("seed memberships");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
         VALUES ($1, $3, 'project'), ($2, $3, 'consuming-project')",
    )
    .bind(fixture.project)
    .bind(fixture.consuming_project)
    .bind(fixture.organization)
    .execute(pool)
    .await
    .expect("seed project");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $3), ($2, $3)",
    )
    .bind(fixture.project)
    .bind(fixture.consuming_project)
    .bind(fixture.maintainer.as_uuid())
    .execute(pool)
    .await
    .expect("seed maintainer");
    sqlx::query(
        "INSERT INTO repositories
         (id, project_id, name, default_branch, is_public)
         VALUES ($1, $3, 'private', 'refs/heads/main', false),
                ($2, $3, 'public', 'refs/heads/main', true),
                ($4, $5, 'consuming', 'refs/heads/main', false)",
    )
    .bind(fixture.private_repository)
    .bind(fixture.public_repository)
    .bind(fixture.project)
    .bind(fixture.consuming_repository)
    .bind(fixture.consuming_project)
    .execute(pool)
    .await
    .expect("seed repositories");
    let family = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let candidate_revision = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'agent')",
    )
    .bind(family)
    .bind(fixture.private_repository)
    .execute(pool)
    .await
    .expect("seed family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(fixture.build)
    .bind(fixture.private_repository)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed build");
    sqlx::query(
        "INSERT INTO build_executions
         (build_request_id, vm_id, release_id, release_agent_id,
          release_version, state, exit_code)
         VALUES ($1, $2, $3, $4, 'v1', 'drafted', 0)",
    )
    .bind(fixture.build)
    .bind(format!("vm-{}", fixture.build))
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .execute(pool)
    .await
    .expect("seed build execution");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5, '{}',
                 $6, $7, 'published', now())",
    )
    .bind(fixture.release)
    .bind(fixture.private_repository)
    .bind("a".repeat(40))
    .bind(fixture.build)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release");
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes,
          media_type, storage_key)
         VALUES ($1, $2, 'bin/agent', 'executable', 365, $3, 1,
                 'application/octet-stream', $4)",
    )
    .bind(fixture.artifact)
    .bind(fixture.release)
    .bind([5_u8; 32].as_slice())
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed release artifact");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'agent', 'Agent', $4, $5, true)",
    )
    .bind(fixture.release_agent)
    .bind(fixture.release)
    .bind(family)
    .bind(json!({
        "command": "bin/agent",
        "arguments": [],
        "working_directory": ".",
        "root_image_digest": "fixture"
    }))
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release agent");
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, 'agent', 'active')",
    )
    .bind(fixture.instance)
    .bind(fixture.consuming_project)
    .bind(family)
    .execute(pool)
    .await
    .expect("seed instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $7, $8, 'fixture/v1', true)",
    )
    .bind(revision)
    .bind(fixture.instance)
    .bind(fixture.release_agent)
    .bind([5_u8; 32].as_slice())
    .bind(json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind(json!({"network": "disabled"}))
    .bind(json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed revision");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $7, $8, 'fixture/v2', true)",
    )
    .bind(candidate_revision)
    .bind(fixture.instance)
    .bind(fixture.release_agent)
    .bind([7_u8; 32].as_slice())
    .bind(json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind(json!({"network": "disabled"}))
    .bind(json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind([8_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed candidate revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(fixture.instance)
        .bind(revision)
        .execute(pool)
        .await
        .expect("activate revision");
    sqlx::query(
        "INSERT INTO runs
         (id, instance_id, instance_revision_id, release_id, release_agent_id,
          run_kind, command_id, state, requires_state, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'update', $6, 'queued', true, now(), now())",
    )
    .bind(fixture.run)
    .bind(fixture.instance)
    .bind(revision)
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed run");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy, created_by)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'manual', $5)",
    )
    .bind(fixture.attachment)
    .bind(fixture.instance)
    .bind(fixture.consuming_project)
    .bind(fixture.consuming_repository)
    .bind(fixture.maintainer.as_uuid())
    .execute(pool)
    .await
    .expect("seed attachment");
    sqlx::query(
        "INSERT INTO agent_updates
         (id, instance_id, expected_current_revision_id,
          candidate_revision_id, state, actor_id)
         VALUES ($1, $2, $3, $4, 'candidate', $5)",
    )
    .bind(fixture.update)
    .bind(fixture.instance)
    .bind(revision)
    .bind(candidate_revision)
    .bind(fixture.maintainer.as_uuid())
    .execute(pool)
    .await
    .expect("seed update");
    sqlx::query(
        "INSERT INTO agent_instance_state_volumes
         (id, instance_id, host_id, host_path, capacity_bytes,
          filesystem_uuid, state)
         VALUES ($1, $2, 'host', $3, 16777216, $4, 'ready')",
    )
    .bind(fixture.volume)
    .bind(fixture.instance)
    .bind(format!("/tmp/{}.raw", fixture.volume))
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed state volume");
    fixture
}
