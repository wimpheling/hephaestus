use identity_domain::UserId;
use sha2::{Digest, Sha256};
use std::error::Error;
use uuid::Uuid;

// The three deliberately related release generations are kept in one fixture
// so their immutable family, artifact, and update-hook differences stay clear.
#[allow(clippy::too_many_lines)]
pub async fn seed_release_catalog(
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
               $8, $9, 'draft', NULL, NULL)",
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
            "build": {
                "image": {"key": "fixture-root"},
                "command": "/bin/sh",
                "arguments": ["-c", "true"],
                "working_directory": "/workspace/source",
                "resources": {"vcpus": 1, "memory_mib": 128},
                "network": {"profile": "disabled"},
                "artifacts": [{
                    "path": "reports/result.txt",
                    "kind": "file",
                    "media_type": "text/plain"
                }],
                "triggers": []
            },
            "guest": {
                "image": {"key": "fixture-root"},
                "command": "bin/browser-reviewer",
                "arguments": [],
                "working_directory": "bin"
            },
            "resources": {"vcpus": 1, "memory_mib": 128},
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
                "resources": {
                    "vcpus": 1,
                    "memory_mib": 128,
                    "network": "broker_only"
                }
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
            "image_reference": "registry.browser.invalid/platform/images/fixture-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "root_image_digest": "registry.browser.invalid/platform/images/fixture-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
