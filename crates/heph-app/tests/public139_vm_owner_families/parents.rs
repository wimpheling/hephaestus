//! Only released parent metadata and a fresh current manager are seeded here.

use forge_domain::ProjectId;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::{ReleaseAgentId, ReleaseId, RuntimePolicy};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::fixture::Fixture;

pub(super) async fn seed(
    pool: &PgPool,
    original: &Fixture,
    policy: &RuntimePolicy,
) -> (AuthenticatedIdentity, ProjectId, ReleaseId, ReleaseAgentId) {
    let project: Uuid = sqlx::query_scalar("SELECT project_id FROM repositories WHERE id=$1")
        .bind(original.repository)
        .fetch_one(pool)
        .await
        .unwrap();
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'Invocation history manager')")
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    let identity = AuthenticatedIdentity::new(
        UserId::from_uuid(user),
        "ordinary-invocation-history",
        user.to_string(),
        serde_json::json!({}),
        RequestId::new(),
    );
    let release = Uuid::new_v4();
    let agent = Uuid::new_v4();
    // This is a declared stateless parent, not evidence of an image or guest run.
    let contract = serde_json::json!({
        "executable":"bin/agent", "arguments":[], "working_directory":".",
        "image_reference":format!("registry.example/history@sha256:{}", "a".repeat(64)),
        "requires_state":false, "publication_mode":"proposal", "policy_ceiling":policy,
        "volume_slots":[]
    });
    let hash: [u8; 32] = Sha256::digest(serde_json::to_vec(&contract).unwrap()).into();
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state,published_at) SELECT $1,repository_id,'invocation-history-v1',source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'published',now() FROM releases WHERE id=$2")
        .bind(release).bind(original.start.release_id.as_uuid()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) SELECT $1,$2,family_id,agent_key,'Invocation history',$3,$4,false FROM release_agents WHERE id=$5")
        .bind(agent).bind(release).bind(contract).bind(hash.as_slice()).bind(original.start.release_agent_id.as_uuid()).execute(pool).await.unwrap();
    (
        identity,
        ProjectId::from_uuid(project),
        ReleaseId::from_uuid(release),
        ReleaseAgentId::from_uuid(agent),
    )
}
