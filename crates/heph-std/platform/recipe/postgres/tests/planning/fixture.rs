use std::collections::BTreeMap;

use forge_domain::ProjectId;
use recipe_application::{
    DeploymentKey, PlanningRequest, PlatformPolicyObservation, RecipePlanner,
};
use recipe_postgres::PostgresPlanningCatalog;
use release_domain::{NetworkAccess, RuntimePolicy};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{fixtures, seed, support::Fixture};

pub const fn policy(memory_mib: u32) -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 1,
        memory_mib,
        network: NetworkAccess::Disabled,
    }
}

pub fn contract() -> Value {
    json!({"executable":"bin/sqlite", "arguments":[], "working_directory":".",
        "image_reference":format!("registry.example/sqlite@sha256:{}", "a".repeat(64)),
        "requires_state":true, "publication_mode":"proposal", "policy_ceiling":policy(128)})
}

pub async fn setup(pool: &PgPool, contract: Value, correct_hash: bool, published: bool) -> Fixture {
    let mut fixture = seed::seed(pool).await;
    // This target exercises authoritative DB facts, not physical backing proof.
    // The shared post-migration seed leaves the provisioning column at reserved.
    sqlx::query("UPDATE agent_instance_state_volumes SET provisioning_state='ready' WHERE id=$1")
        .bind(fixture.volume)
        .execute(pool)
        .await
        .expect("ready metadata fixture");
    let release = Uuid::new_v4();
    let agent = Uuid::new_v4();
    let bytes = serde_json::to_vec(&contract).unwrap();
    let hash: [u8; 32] = if correct_hash {
        Sha256::digest(bytes).into()
    } else {
        [0; 32]
    };
    sqlx::query("INSERT INTO releases (id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state)
        SELECT $1,repository_id,'planner-v1',source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'draft'
        FROM releases WHERE id=$2")
        .bind(release).bind(fixture.release).execute(pool).await.expect("author planning draft");
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state)
        SELECT $1,$2,family_id,'planner','Planner',$3,$4,true FROM release_agents WHERE id=$5")
        .bind(agent).bind(release).bind(contract).bind(hash.as_slice()).bind(fixture.release_agent)
        .execute(pool).await.expect("author exact source");
    if published {
        sqlx::query("UPDATE releases SET state='published',published_at=now() WHERE id=$1")
            .bind(release)
            .execute(pool)
            .await
            .expect("publish planning source");
    }
    fixture.release = release;
    fixture.release_agent = agent;
    fixture
}

pub fn request(fixture: &Fixture, external: bool) -> PlanningRequest {
    PlanningRequest::new(
        ProjectId::from_uuid(fixture.consuming_project),
        DeploymentKey::parse("sqlite").unwrap(),
        fixtures::source(fixture, external).as_bytes(),
        BTreeMap::new(),
        if external {
            BTreeMap::from([(
                fixtures::key("data"),
                runtime_types::VolumeId::from_uuid(fixture.volume),
            )])
        } else {
            BTreeMap::new()
        },
    )
    .unwrap()
}

pub async fn planner(pool: &PgPool, memory_mib: u32) -> RecipePlanner<PostgresPlanningCatalog> {
    RecipePlanner::new(PostgresPlanningCatalog::new(
        fixtures::app_pool(pool).await,
        PlatformPolicyObservation::new(policy(memory_mib), String::from("platform/planning-v1"))
            .unwrap(),
    ))
}
