use forge_domain::RepositoryId;
use forge_service::GitStorage;
use identity_domain::UserId;
use review_domain::ReviewProposalId;
use runtime_types::RunId;
use sqlx::PgPool;
use tempfile::TempDir;
use uuid::Uuid;

use super::git::{git_text, run_git};

pub struct Fixture {
    pub actor_id: UserId,
    pub repository_id: RepositoryId,
    pub run_id: RunId,
    pub proposal_id: ReviewProposalId,
    pub input_commit: String,
    pub result_commit: String,
    pub instance_id: Uuid,
    pub instance_revision_id: Uuid,
    pub release_id: Uuid,
    pub release_agent_id: Uuid,
    pub attachment_id: Uuid,
}

// Keeping the relational and Git fixture together makes its provenance
// invariant visible to the approval tests.
#[allow(clippy::too_many_lines)]
pub async fn seed(pool: &PgPool, storage: &GitStorage, temporary: &TempDir) -> Fixture {
    let actor_id = UserId::new();
    let organization_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let repository_id = RepositoryId::new();
    let family_id = Uuid::new_v4();
    let build_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let instance_revision_id = Uuid::new_v4();
    let attachment_id = Uuid::new_v4();
    let run_id = RunId::new();
    let receive_id = Uuid::new_v4();
    let result_id = Uuid::new_v4();
    let proposal_id = ReviewProposalId::new();

    storage
        .create_bare(repository_id, "main")
        .await
        .expect("create bare repository");
    let work = temporary.path().join("work");
    run_git(
        temporary.path(),
        &["init", work.to_str().expect("UTF-8 path")],
    );
    run_git(&work, &["config", "user.name", "Fixture"]);
    run_git(&work, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(work.join("README.md"), "input\n").expect("write input");
    run_git(&work, &["add", "README.md"]);
    run_git(&work, &["commit", "-m", "input"]);
    let input_commit = git_text(&work, &["rev-parse", "HEAD"]);
    run_git(
        &work,
        &[
            "remote",
            "add",
            "origin",
            storage
                .repository_path(repository_id)
                .to_str()
                .expect("UTF-8 repository"),
        ],
    );
    run_git(&work, &["push", "origin", "HEAD:refs/heads/main"]);
    std::fs::write(work.join("README.md"), "input\nagent result\n").expect("write result");
    run_git(&work, &["commit", "-am", "result"]);
    let result_commit = git_text(&work, &["rev-parse", "HEAD"]);
    let result_ref = format!("refs/heads/hephaestus/{instance_id}/{run_id}");
    run_git(&work, &["push", "origin", &format!("HEAD:{result_ref}")]);

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Reviewer')")
        .bind(actor_id.as_uuid())
        .execute(pool)
        .await
        .expect("user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'Fixture')")
        .bind(organization_id)
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("owner membership");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
         VALUES ($1, $2, 'project')",
    )
    .bind(project_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("project");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $2)",
    )
    .bind(project_id)
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("project maintainer");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, 'repository')",
    )
    .bind(repository_id.as_uuid())
    .bind(project_id)
    .execute(pool)
    .await
    .expect("repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'reviewer')",
    )
    .bind(family_id)
    .bind(repository_id.as_uuid())
    .execute(pool)
    .await
    .expect("agent family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build_id)
    .bind(repository_id.as_uuid())
    .bind(&input_commit)
    .bind([1_u8; 32].as_slice())
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, publication_actor_id,
          published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5, '{}',
                 $6, $7, 'published', $8, now())",
    )
    .bind(release_id)
    .bind(repository_id.as_uuid())
    .bind(&input_commit)
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'reviewer', 'Reviewer', $4, $5, '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(serde_json::json!({
        "command": "bin/reviewer",
        "arguments": [],
        "working_directory": ".",
        "root_image_digest": "fixture"
    }))
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("release agent");
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state, created_by)
         VALUES ($1, $2, $3, 'reviewer', 'active', $4)",
    )
    .bind(instance_id)
    .bind(project_id)
    .bind(family_id)
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable, created_by)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $7, $8,
                 'fixture/v1', true, $9)",
    )
    .bind(instance_revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind(serde_json::json!({"network": "disabled"}))
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind([6_u8; 32].as_slice())
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("instance revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(instance_revision_id)
        .execute(pool)
        .await
        .expect("activate revision");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy, created_by)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', $5)",
    )
    .bind(attachment_id)
    .bind(instance_id)
    .bind(project_id)
    .bind(repository_id.as_uuid())
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("attachment");
    sqlx::query(
        "INSERT INTO runs
         (id, instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, command_id, state, outcome, requires_state,
          created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'normal', $7,
                 'cleaned_up', 'succeeded', false, now(), now())",
    )
    .bind(run_id.as_uuid())
    .bind(instance_id)
    .bind(instance_revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(attachment_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("run");
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'fixture', 'accepted', now())",
    )
    .bind(receive_id)
    .bind(repository_id.as_uuid())
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("receive");
    sqlx::query(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, receive_id,
          run_id, command_id, actor_id, request_id, instance_id,
          instance_revision_id, release_id, release_agent_id, attachment_id,
          platform_policy_version, requires_state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, $6, $7, $8,
                 $9, $10, $11, $12, $13, 'fixture/v1', false)",
    )
    .bind(Uuid::new_v4())
    .bind(repository_id.as_uuid())
    .bind(&input_commit)
    .bind(receive_id)
    .bind(run_id.as_uuid())
    .bind(Uuid::new_v4())
    .bind(actor_id.as_uuid())
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(instance_revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(attachment_id)
    .execute(pool)
    .await
    .expect("run request");
    sqlx::query(
        "INSERT INTO run_results
         (id, run_id, repository_id, instance_id, input_commit, result_commit,
          result_ref, message, state, completed_at, instance_revision_id,
          release_id, release_agent_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'fixture result',
                 'completed', now(), $8, $9, $10)",
    )
    .bind(result_id)
    .bind(run_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(instance_id)
    .bind(&input_commit)
    .bind(&result_commit)
    .bind(&result_ref)
    .bind(instance_revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .execute(pool)
    .await
    .expect("result");
    let generated_proposal: Uuid =
        sqlx::query_scalar("SELECT id FROM review_proposals WHERE result_id = $1")
            .bind(result_id)
            .fetch_one(pool)
            .await
            .expect("generated proposal");
    sqlx::query("UPDATE review_proposals SET id = $2 WHERE id = $1")
        .bind(generated_proposal)
        .bind(proposal_id.as_uuid())
        .execute(pool)
        .await
        .expect("stable proposal ID");

    Fixture {
        actor_id,
        repository_id,
        run_id,
        proposal_id,
        input_commit,
        result_commit,
        instance_id,
        instance_revision_id,
        release_id,
        release_agent_id,
        attachment_id,
    }
}
