use std::collections::BTreeMap;

use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentId, DeploymentIntent, DeploymentKey, DeploymentOperation,
    InstallDeployment, RemoveDeployment,
};
use recipe_domain::{ExternalVolume, ReleaseCatalogEntry, ReleasePin};
use release_domain::{ParameterName, ParameterValue, ReleaseAgentId, ReleaseId};
use runtime_types::VolumeId;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::support::Fixture;

pub async fn app_pool(pool: &PgPool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .expect("app role pool")
}

pub fn key(value: &str) -> CapabilitySlotKey {
    CapabilitySlotKey::parse(value).expect("resource key")
}

pub fn source(fixture: &Fixture, external: bool) -> String {
    let volume = if external {
        "source = { type = \"external\" }"
    } else {
        "source = { type = \"created\", capacity_bytes = { source = \"input\", name = \"capacity\" } }"
    };
    format!(
        r#"
contract_version = 1
recipe_id = "database"
recipe_version = "1.0.0"
[[inputs]]
name = "capacity"
default = 16777216
value_type = {{ type = "integer", minimum = 16777216, maximum = 1073741824 }}
[[resources]]
kind = "volume"
name = "data"
{volume}
[[resources]]
kind = "instance"
name = "sqlite"
release = {{ release_id = "{}", release_agent_id = "{}" }}
volume_bindings = [{{ slot = "state", resource = "data", guest_path = "/var/lib/hephaestus", access_mode = "read_write" }}]
removal = "delete"
"#,
        fixture.release, fixture.release_agent
    )
}

pub fn catalog(fixture: &Fixture) -> ReleaseCatalogEntry {
    ReleaseCatalogEntry {
        pin: ReleasePin {
            release_id: ReleaseId::from_uuid(fixture.release),
            release_agent_id: ReleaseAgentId::from_uuid(fixture.release_agent),
        },
        published: true,
        parameters: Vec::new(),
        volume_slots: release_domain::effective_volume_slots(&[], true).expect("legacy slots"),
        required_secret_slot_count: 0,
        capability_requirements: Vec::new(),
    }
}

pub fn intent(fixture: &Fixture, external: bool) -> DeploymentIntent {
    build(
        fixture,
        &source(fixture, external),
        DeploymentId::from_uuid(Uuid::new_v4()).expect("deployment"),
        "sqlite",
        &BTreeMap::new(),
        &[catalog(fixture)],
        external,
    )
}

pub fn build(
    fixture: &Fixture,
    source: &str,
    id: DeploymentId,
    key: &str,
    inputs: &BTreeMap<ParameterName, ParameterValue>,
    catalog: &[ReleaseCatalogEntry],
    external: bool,
) -> DeploymentIntent {
    let parsed = recipe_domain::parse_recipe(source.as_bytes()).expect("valid declaration");
    let external = if external {
        BTreeMap::from([(
            self::key("data"),
            ExternalVolume {
                id: VolumeId::from_uuid(fixture.volume),
                capacity_bytes: 16_777_216,
            },
        )])
    } else {
        BTreeMap::new()
    };
    let resolved = parsed
        .resolve(inputs, &external, catalog)
        .expect("caller resolution");
    DeploymentIntent::new(
        id,
        ProjectId::from_uuid(fixture.consuming_project),
        DeploymentKey::parse(key).expect("deployment key"),
        &parsed,
        &resolved,
    )
    .expect("intent")
}

pub fn install(identity: &AuthenticatedIdentity, intent: DeploymentIntent) -> InstallDeployment {
    InstallDeployment {
        command: CommandIdentity::from_identity(identity, DeploymentOperation::Install)
            .expect("command"),
        intent,
    }
}

pub fn remove(
    identity: &AuthenticatedIdentity,
    id: DeploymentId,
    expected_version: u64,
) -> RemoveDeployment {
    RemoveDeployment {
        command: CommandIdentity::from_identity(identity, DeploymentOperation::Remove)
            .expect("remove command"),
        deployment_id: id,
        expected_version,
    }
}

pub async fn required_secret_release(pool: &PgPool, fixture: &Fixture) -> Fixture {
    let mut fixture = *fixture;
    let release = Uuid::new_v4();
    let agent = Uuid::new_v4();
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref,
        build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state)
        SELECT $1, repository_id, 'with-secret', source_commit, source_ref, build_request_id,
            build_definition_hash, configuration, configuration_hash, manifest_hash, 'draft'
        FROM releases WHERE id = $2").bind(release).bind(fixture.release).execute(pool).await.expect("draft release");
    sqlx::query(
        "INSERT INTO release_agents (id, release_id, family_id, agent_key, display_name,
        runtime_contract, runtime_contract_hash, requires_state, secret_slot_schema)
        SELECT $1, $2, family_id, agent_key, display_name, runtime_contract, runtime_contract_hash,
            requires_state, $3 FROM release_agents WHERE id = $4",
    )
    .bind(agent)
    .bind(release)
    .bind(serde_json::json!([{
        "key": "credential", "purpose": "required fixture slot", "required": true,
        "delivery_modes": ["raw"], "phases": ["normal"], "destinations": [],
    }]))
    .bind(fixture.release_agent)
    .execute(pool)
    .await
    .expect("author secret schema in draft");
    sqlx::query("UPDATE releases SET state = 'published', published_at = now() WHERE id = $1")
        .bind(release)
        .execute(pool)
        .await
        .expect("publish exact secret catalog");
    fixture.release = release;
    fixture.release_agent = agent;
    fixture
}
