use super::*;

pub(crate) async fn seed_update_release(
    pool: &PgPool,
    current_release_id: ReleaseId,
    current_release_agent_id: ReleaseAgentId,
) -> ReleaseAgentId {
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         SELECT $1, repository_id, 'v2.0.0', source_commit, source_ref,
                build_request_id, build_definition_hash, configuration,
                configuration_hash, manifest_hash, 'published', now()
         FROM releases WHERE id = $2",
    )
    .bind(release_id.as_uuid())
    .bind(current_release_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed update release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state, update_hook)
         SELECT $1, $2, family_id, agent_key, display_name,
                runtime_contract, $3, parameter_schema, '[]',
                requires_state, $4
         FROM release_agents WHERE id = $5",
    )
    .bind(release_agent_id.as_uuid())
    .bind(release_id.as_uuid())
    .bind([42_u8; 32].as_slice())
    .bind(json!({
        "command": "bin/update",
        "arguments": [],
        "timeout_seconds": 60
    }))
    .bind(current_release_agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed update release agent");
    release_agent_id
}

pub(crate) async fn seed_fork_release(
    pool: &PgPool,
    current_release_id: ReleaseId,
    current_release_agent_id: ReleaseAgentId,
    fork_repository_id: RepositoryId,
) -> ReleaseAgentId {
    let build_id = BuildRequestId::new();
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let family_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id.as_uuid())
    .bind(fork_repository_id.as_uuid())
    .bind("d".repeat(40))
    .bind([71_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed fork build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         SELECT $1, $2, 'v1.0.0-fork', $3, source_ref,
                $4, $5, configuration, configuration_hash,
                manifest_hash, 'published', now()
         FROM releases WHERE id = $6",
    )
    .bind(release_id.as_uuid())
    .bind(fork_repository_id.as_uuid())
    .bind("d".repeat(40))
    .bind(build_id.as_uuid())
    .bind([71_u8; 32].as_slice())
    .bind(current_release_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed fork release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         SELECT $1, $2, agent_key
         FROM release_agents WHERE id = $3",
    )
    .bind(family_id)
    .bind(fork_repository_id.as_uuid())
    .bind(current_release_agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed fork family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state, update_hook)
         SELECT $1, $2, $3, agent_key, display_name,
                runtime_contract, $4, parameter_schema, secret_slot_schema,
                requires_state, update_hook
         FROM release_agents WHERE id = $5",
    )
    .bind(release_agent_id.as_uuid())
    .bind(release_id.as_uuid())
    .bind(family_id)
    .bind([72_u8; 32].as_slice())
    .bind(current_release_agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed fork release agent");
    release_agent_id
}

pub(crate) fn reusable_config() -> String {
    r#"
version = 2
[agent]
name = "Reviewer"
key = "reviewer"
[build]
image = { key = "build" }
command = "/bin/build"
working_directory = "/source"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 2
memory_mib = 1024
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "bin/reviewer"
kind = "executable"
[guest]
image = { key = "runtime" }
command = "bin/reviewer"
arguments = ["--json"]
working_directory = "bin"
[resources]
vcpus = 4
memory_mib = 2048
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = true
[network]
profile = "egress"
[triggers]
push = false
[[parameters]]
name = "severity"
pub type = "enum"
values = ["warning", "error"]
required = true
[[secret_slots]]
key = "model"
purpose = "Call a configured model"
required = true
delivery_modes = ["brokered"]
phases = ["normal"]
destinations = ["api.example.test"]
"#
    .to_owned()
}

pub(crate) fn brokered_config() -> String {
    format!(
        "{}\n[[parameters]]\nname = \"model_rule_id\"\ntype = \"string\"\nminimum_length = 36\nmaximum_length = 36\nrequired = true\n[[parameters]]\nname = \"relay_rule_id\"\ntype = \"string\"\nminimum_length = 36\nmaximum_length = 36\nrequired = true\n[[secret_slots]]\nkey = \"relay\"\npurpose = \"Second brokered test authority\"\nrequired = true\ndelivery_modes = [\"brokered\"]\nphases = [\"normal\"]\ndestinations = [\"relay.example\"]\n",
        reusable_config()
    )
}

pub(crate) fn broker_copy_parameters(
    severity: &str,
    model_rule_id: Uuid,
    relay_rule_id: Uuid,
) -> BTreeMap<ParameterName, ParameterValue> {
    BTreeMap::from([
        (
            ParameterName::parse("severity").expect("parameter name"),
            ParameterValue::String(severity.to_owned()),
        ),
        (
            ParameterName::parse("model_rule_id").expect("parameter name"),
            ParameterValue::String(model_rule_id.to_string()),
        ),
        (
            ParameterName::parse("relay_rule_id").expect("parameter name"),
            ParameterValue::String(relay_rule_id.to_string()),
        ),
    ])
}

pub(crate) fn secret_key(operation: &str, id: Uuid) -> secret_domain::SecretCommandKey {
    secret_domain::SecretCommandKey::derive(operation, &[id.as_bytes()])
}

pub(crate) async fn seed_matching_update_release(
    pool: &PgPool,
    current_release_id: ReleaseId,
    current_release_agent_id: ReleaseAgentId,
) -> ReleaseAgentId {
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         SELECT $1, repository_id, $2, source_commit, source_ref,
                build_request_id, build_definition_hash, configuration,
                configuration_hash, manifest_hash, 'published', now()
         FROM releases WHERE id = $3",
    )
    .bind(release_id.as_uuid())
    .bind(format!("broker-copy-{release_id}"))
    .bind(current_release_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed matching candidate release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state, update_hook)
         SELECT $1, $2, family_id, agent_key, display_name,
                runtime_contract, $3, parameter_schema,
                secret_slot_schema, requires_state, update_hook
         FROM release_agents WHERE id = $4",
    )
    .bind(release_agent_id.as_uuid())
    .bind(release_id.as_uuid())
    .bind([88_u8; 32].as_slice())
    .bind(current_release_agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed matching candidate agent");
    release_agent_id
}
