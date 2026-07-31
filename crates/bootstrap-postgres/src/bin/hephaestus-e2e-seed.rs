//! Seeds the deterministic browser E2E identity and empty bare repository.

use forge_domain::{GitRef, OrganizationId, ProjectId, Repository, RepositoryId};
use forge_postgres::PgForgeRepository;
use forge_service::{CreateRepository, GitStorage};
use identity_domain::UserId;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::{env, error::Error, path::PathBuf, sync::Arc};
use uuid::Uuid;

const USER_ID: &str = "10000000-0000-4000-8000-000000000001";
const ORGANIZATION_ID: &str = "10000000-0000-4000-8000-000000000002";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let schema_only = env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "--schema-only");
    let database_url = env::var("HEPHAESTUS_DATABASE_URL")?;
    let repository_root = PathBuf::from(env::var("HEPHAESTUS_REPOSITORY_ROOT")?);
    let artifact_root = PathBuf::from(env::var("HEPHAESTUS_ARTIFACT_ROOT")?);
    let issuer = env::var("HEPHAESTUS_BROWSER_OIDC_ISSUER")
        .unwrap_or_else(|_| String::from("http://127.0.0.1:5556"));
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    if schema_only {
        return Ok(());
    }

    let user_id = UserId::from_uuid(Uuid::parse_str(USER_ID)?);
    let organization_id = OrganizationId::from_uuid(Uuid::parse_str(ORGANIZATION_ID)?);
    sqlx::query(
        "INSERT INTO users (id, display_name)
           VALUES ($1, 'Ada Reviewer')
           ON CONFLICT (id) DO NOTHING",
    )
    .bind(user_id.as_uuid())
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO external_identities
           (user_id, issuer, subject, provider_metadata)
           VALUES ($1, $2, 'reviewer', '{\"fixture\":true}'::jsonb)
           ON CONFLICT (issuer, subject) DO NOTHING",
    )
    .bind(user_id.as_uuid())
    .bind(&issuer)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO organizations (id, name)
           VALUES ($1, 'Acme Research')
           ON CONFLICT (id) DO NOTHING",
    )
    .bind(organization_id.as_uuid())
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
           VALUES ($1, $2, 'owner')
           ON CONFLICT (organization_id, user_id) DO UPDATE SET role = 'owner'",
    )
    .bind(organization_id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(&pool)
    .await?;

    let storage = Arc::new(GitStorage::initialize(&repository_root).await?);
    let forge = PgForgeRepository::new(pool.clone(), Arc::clone(&storage));
    let (project_id, repository) = bootstrap_forge(&pool, &forge, organization_id, user_id).await?;
    seed_secret_roles(
        &pool,
        project_id.as_uuid(),
        repository.id.as_uuid(),
        user_id,
    )
    .await?;
    let release_agents =
        seed_release_catalog(&pool, repository.id.as_uuid(), user_id, &artifact_root).await?;

    println!(
        "{}",
        serde_json::json!({
            "user_id": user_id,
            "organization_id": organization_id,
            "project_id": project_id,
            "repository_id": repository.id,
            "release_agents": release_agents,
        })
    );
    Ok(())
}

async fn seed_secret_roles(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    repository_id: Uuid,
    user_id: UserId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO project_secret_roles (project_id, user_id, role)
           VALUES ($1, $2, 'secret_manager')
           ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(user_id.as_uuid())
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO repository_secret_roles (repository_id, user_id, role)
           VALUES ($1, $2, 'secret_manager')
           ON CONFLICT DO NOTHING",
    )
    .bind(repository_id)
    .bind(user_id.as_uuid())
    .execute(pool)
    .await?;
    Ok(())
}

// The three deliberately related release generations are kept in one fixture
// so their immutable family, artifact, and update-hook differences stay clear.
#[allow(clippy::too_many_lines)]
async fn seed_release_catalog(
    pool: &sqlx::PgPool,
    repository_id: Uuid,
    user_id: UserId,
    artifact_root: &std::path::Path,
) -> Result<Vec<Uuid>, Box<dyn Error>> {
    let existing = sqlx::query_scalar::<_, Uuid>(
        "SELECT release_agent.id
           FROM release_agents release_agent
           JOIN releases release ON release.id = release_agent.release_id
           WHERE release.repository_id = $1
           ORDER BY release.version",
    )
    .bind(repository_id)
    .fetch_all(pool)
    .await?;
    if existing.len() >= 3 {
        return Ok(existing);
    }

    let release_root = artifact_root.join("releases");
    tokio::fs::create_dir_all(&release_root).await?;
    let family_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
           VALUES ($1, $2, 'browser-reviewer')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await?;

    let mut release_agents = Vec::new();
    for (index, version) in ["v1", "v2", "v3-failing"].into_iter().enumerate() {
        let build_id = Uuid::new_v4();
        let release_id = Uuid::new_v4();
        let release_agent_id = Uuid::new_v4();
        let artifact_id = Uuid::new_v4();
        let storage_key = Uuid::new_v4();
        let source_commit = format!("{:040x}", index + 1);
        let artifact = format!("#!/bin/sh\n# browser fixture {version}\nexit 0\n");
        tokio::fs::write(
            release_root.join(storage_key.simple().to_string()),
            artifact.as_bytes(),
        )
        .await?;
        let artifact_hash: [u8; 32] = Sha256::digest(artifact.as_bytes()).into();
        sqlx::query(
            "INSERT INTO build_requests
               (id, repository_id, source_commit, source_ref,
                build_definition_hash, state, created_by, completed_at)
               VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5, now())",
        )
        .bind(build_id)
        .bind(repository_id)
        .bind(&source_commit)
        .bind(Sha256::digest(format!("build-{version}")).as_slice())
        .bind(user_id.as_uuid())
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO releases
               (id, repository_id, version, source_commit, source_ref,
                build_request_id, build_definition_hash, configuration,
                configuration_hash, manifest_hash, state,
                publication_actor_id, published_at)
               VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, $7,
                       $8, $9, 'published', $10, now())",
        )
        .bind(release_id)
        .bind(repository_id)
        .bind(version)
        .bind(&source_commit)
        .bind(build_id)
        .bind(Sha256::digest(format!("build-{version}")).as_slice())
        .bind(serde_json::json!({
            "version": 2,
            "agent": {
                "name": "Reusable reviewer",
                "key": "browser-reviewer"
            },
            "guest": {
                "command": "bin/browser-reviewer",
                "arguments": [],
                "working_directory": "bin"
            },
            "resources": {"vcpus": 1, "memory_mib": 128},
            "root_image": {"reference": "fixture-root@sha256:e2e"},
            "workspace": {
                "mount": true,
                "path": "/workspace/repo",
                "read_only": true
            },
            "state_volume": {"enabled": true},
            "results": {"declared_files": ["reports/result.txt"]},
            "network": {"profile": "broker_only"},
            "triggers": {"push": true, "refs": ["refs/heads/main"]}
        }))
        .bind(Sha256::digest(format!("config-{version}")).as_slice())
        .bind(Sha256::digest(format!("manifest-{version}")).as_slice())
        .bind(user_id.as_uuid())
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO release_artifacts
               (id, release_id, path, kind, mode, content_hash, size_bytes,
                media_type, storage_key)
               VALUES ($1, $2, 'bin/browser-reviewer', 'executable', 365,
                       $3, $4, 'application/octet-stream', $5)",
        )
        .bind(artifact_id)
        .bind(release_id)
        .bind(artifact_hash.as_slice())
        .bind(i64::try_from(artifact.len())?)
        .bind(storage_key)
        .execute(pool)
        .await?;
        let update_hook = if index == 0 {
            None
        } else {
            Some(serde_json::json!({
                "command": "bin/browser-reviewer",
                "arguments": if index == 2 {
                    vec![String::from("uncertain")]
                } else {
                    Vec::<String>::new()
                },
                "timeout_seconds": 30,
                "resources": {"vcpus": 1, "memory_mib": 128}
            }))
        };
        sqlx::query(
            "INSERT INTO release_agents
               (id, release_id, family_id, agent_key, display_name,
                runtime_contract, runtime_contract_hash, parameter_schema,
                secret_slot_schema, requires_state, update_hook)
               VALUES ($1, $2, $3, 'browser-reviewer', 'Reusable reviewer',
                       $4, $5, $6, $7, true, $8)",
        )
        .bind(release_agent_id)
        .bind(release_id)
        .bind(family_id)
        .bind(serde_json::json!({
            "executable": "bin/browser-reviewer",
            "arguments": [],
            "working_directory": "bin",
            "root_image_digest": "fixture-root@sha256:e2e",
            "requires_state": true,
            "policy_ceiling": {
                "vcpus": 1,
                "memory_mib": 128,
                "network": "broker_only"
            }
        }))
        .bind(Sha256::digest(format!("contract-{version}")).as_slice())
        .bind(serde_json::json!([
            {
                "name": "review_style",
                "value_type": {"type": "enum", "values": ["strict", "balanced"]},
                "required": true,
                "default": "balanced",
                "sensitive": false
            },
            {
                "name": "private_hint",
                "value_type": {
                    "type": "string",
                    "minimum_length": 0,
                    "maximum_length": 128
                },
                "required": false,
                "default": "",
                "sensitive": true
            }
        ]))
        .bind(serde_json::json!([
            {
                "key": "raw_token",
                "purpose": "Read-only fixture credential file",
                "required": false,
                "delivery_modes": ["raw"],
                "phases": ["normal"],
                "destinations": []
            },
            {
                "key": "broker_token",
                "purpose": "Host-side semantic API operation",
                "required": false,
                "delivery_modes": ["brokered"],
                "phases": ["normal", "update"],
                "destinations": ["api.example.com"]
            }
        ]))
        .bind(update_hook)
        .execute(pool)
        .await?;
        release_agents.push(release_agent_id);
    }
    Ok(release_agents)
}

async fn bootstrap_forge(
    pool: &sqlx::PgPool,
    forge: &PgForgeRepository,
    organization_id: OrganizationId,
    user_id: UserId,
) -> Result<(ProjectId, Repository), Box<dyn Error>> {
    let project_id = match sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM projects WHERE organization_id = $1 AND name = 'autonomy-lab'",
    )
    .bind(organization_id.as_uuid())
    .fetch_optional(pool)
    .await?
    {
        Some(id) => ProjectId::from_uuid(id),
        None => {
            forge
                .create_project_trusted(organization_id, "autonomy-lab")
                .await?
                .id
        }
    };
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
           VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(project_id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(pool)
    .await?;
    let repository = match sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM repositories WHERE project_id = $1 AND name = 'agent-workbench'",
    )
    .bind(project_id.as_uuid())
    .fetch_optional(pool)
    .await?
    {
        Some(id) => forge.get_repository(RepositoryId::from_uuid(id)).await?,
        None => {
            forge
                .create_repository_trusted(&CreateRepository {
                    project_id,
                    name: String::from("agent-workbench"),
                    default_branch: GitRef::parse("refs/heads/main")?,
                    is_public: false,
                    agent_runs_enabled: true,
                })
                .await?
        }
    };
    Ok((project_id, repository))
}
