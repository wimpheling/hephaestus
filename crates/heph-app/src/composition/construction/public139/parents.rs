//! Declared stateless released parents and current management authorization.
//!
//! No Run, receipt, runtime session, positive drain or command record is seeded.

use crate::RuntimePolicy;
use heph_forge::{CommitSha, GitRef, ProjectId, Repository, RepositoryId};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use runtime_types::AgentInstanceId;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use time::OffsetDateTime;
use uuid::Uuid;

pub(super) fn repository() -> Repository {
    Repository {
        id: RepositoryId::new(),
        project_id: ProjectId::new(),
        name: "ordinary-public139".into(),
        default_branch: GitRef::parse("refs/heads/main").expect("branch"),
        is_public: false,
        agent_runs_enabled: true,
        created_at: OffsetDateTime::now_utc(),
    }
}

pub(super) async fn seed(
    pool: &PgPool,
    artifacts: &release_artifact_store::LocalArtifactStore,
    repository: &Repository,
    commit: &CommitSha,
    policy: &RuntimePolicy,
    image: &str,
    root: &Path,
) -> (AuthenticatedIdentity, AgentInstanceId) {
    let actor = identity(pool, repository).await;
    let release = Uuid::new_v4();
    let family = Uuid::new_v4();
    let agent = Uuid::new_v4();
    released(pool, repository, commit, release, family).await;
    let contract = json!({"executable":"bin/agent", "arguments":[], "working_directory":"bin",
        "image_reference":image, "requires_state":false, "publication_mode":"proposal",
        "volume_slots":[], "policy_ceiling":{"vcpus":1,"memory_mib":512,"network":"disabled"}});
    let hash: [u8; 32] = Sha256::digest(serde_json::to_vec(&contract).unwrap()).into();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) VALUES($1,$2,$3,'runtime','Ordinary public139',$4,$5,false)")
        .bind(agent).bind(release).bind(family).bind(contract).bind(hash.as_slice())
        .execute(pool).await.expect("declared stateless release agent");
    artifact(pool, artifacts, release, root).await;
    let instance = AgentInstanceId::new();
    let revision = Uuid::new_v4();
    let effective = json!({"vcpus":1,"memory_mib":512,"network":"disabled"});
    let hash: [u8; 32] = Sha256::digest(serde_json::to_vec(&effective).unwrap()).into();
    sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state) VALUES($1,$2,$3,'ordinary','active')")
        .bind(instance.as_uuid()).bind(repository.project_id.as_uuid()).bind(family)
        .execute(pool).await.expect("declared ordinary instance");
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) VALUES($1,$2,$3,'{}',$4,'{}','{}',$5,$6,$7,true)")
        .bind(revision).bind(instance.as_uuid()).bind(agent).bind(Sha256::digest(b"{}").as_slice())
        .bind(effective).bind(hash.as_slice()).bind(&policy.version)
        .execute(pool).await.expect("declared immutable revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
        .bind(instance.as_uuid())
        .bind(revision)
        .execute(pool)
        .await
        .expect("parent active revision");
    sqlx::query("INSERT INTO agent_attachments(id,instance_id,project_id,repository_id,ref_selector,trigger_policy) VALUES($1,$2,$3,$4,'refs/heads/main','push')")
        .bind(Uuid::new_v4()).bind(instance.as_uuid()).bind(repository.project_id.as_uuid())
        .bind(repository.id.as_uuid()).execute(pool).await.expect("actual push target");
    (actor, instance)
}

async fn identity(pool: &PgPool, repository: &Repository) -> AuthenticatedIdentity {
    let organization = Uuid::new_v4();
    let user = UserId::new();
    sqlx::query("INSERT INTO organizations(id,name) VALUES($1,$2)")
        .bind(organization)
        .bind(format!("ordinary139-{organization}"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO projects(id,organization_id,name) VALUES($1,$2,'ordinary139')")
        .bind(repository.project_id.as_uuid())
        .bind(organization)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'Ordinary139 manager')")
        .bind(user.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(repository.project_id.as_uuid())
        .bind(user.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO repositories(id,project_id,name) VALUES($1,$2,'ordinary-public139')")
        .bind(repository.id.as_uuid())
        .bind(repository.project_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    AuthenticatedIdentity::new(
        user,
        "ordinary139",
        user.to_string(),
        json!({}),
        RequestId::new(),
    )
}

async fn released(
    pool: &PgPool,
    repository: &Repository,
    commit: &CommitSha,
    release: Uuid,
    family: Uuid,
) {
    let build = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_families(id,repository_id,agent_key) VALUES($1,$2,'runtime')")
        .bind(family)
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    // A declared released parent, not an assertion that this test ran a builder.
    sqlx::query("INSERT INTO build_requests(id,repository_id,source_commit,source_ref,build_definition_hash,state,completed_at) VALUES($1,$2,$3,'refs/heads/main',$4,'succeeded',now())")
        .bind(build).bind(repository.id.as_uuid()).bind(commit.as_str()).bind([1_u8;32].as_slice())
        .execute(pool).await.unwrap();
    let configuration = configuration();
    let configuration_hash: [u8; 32] =
        Sha256::digest(serde_json::to_vec(&configuration).unwrap()).into();
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state,published_at) VALUES($1,$2,'ordinary139-v1',$3,'refs/heads/main',$4,$5,$6,$7,$8,'published',now())")
        .bind(release).bind(repository.id.as_uuid()).bind(commit.as_str()).bind(build)
        .bind([1_u8;32].as_slice()).bind(configuration).bind(configuration_hash.as_slice()).bind([3_u8;32].as_slice())
        .execute(pool).await.unwrap();
}

async fn artifact(
    pool: &PgPool,
    store: &release_artifact_store::LocalArtifactStore,
    release: Uuid,
    root: &Path,
) {
    let sealed = root.join("sealed");
    fs::create_dir(&sealed).unwrap();
    fs::create_dir(sealed.join("bin")).unwrap();
    fs::write(sealed.join("bin/agent"), b"#!/bin/sh\nsleep 300\n").unwrap();
    fs::set_permissions(sealed.join("bin/agent"), fs::Permissions::from_mode(0o555)).unwrap();
    let artifacts = store
        .import(&sealed)
        .expect("actual canonical artifact import");
    assert_eq!(artifacts.len(), 1);
    let a = &artifacts[0];
    sqlx::query("INSERT INTO release_artifacts(id,release_id,path,kind,mode,content_hash,size_bytes,media_type,storage_key) VALUES($1,$2,$3,'executable',$4,$5,$6,'application/octet-stream',$7)")
        .bind(Uuid::new_v4()).bind(release).bind(a.path.as_str()).bind(i32::from(a.mode))
        .bind(a.content_hash.as_bytes().as_slice()).bind(i64::try_from(a.size_bytes).unwrap())
        .bind(a.storage_key).execute(pool).await.expect("exact actually imported parent artifact");
}

fn configuration() -> serde_json::Value {
    let value = json!({
        "version":2, "agent":{"name":"Ordinary139","key":"runtime"},
        "build":{"image":{"key":"ordinary139"},"command":"/bin/sh","arguments":[],
            "working_directory":"/workspace","resources":{"vcpus":1,"memory_mib":512},
            "network":{"profile":"disabled"},"artifacts":[{"path":"bin/agent","kind":"executable"}],"triggers":[]},
        "guest":{"image":{"key":"ordinary139"},"command":"bin/agent","arguments":[],"working_directory":"bin"},
        "resources":{"vcpus":1,"memory_mib":512},
        "workspace":{"mount":true,"path":"/workspace/repo","read_only":true},
        "publication":{"mode":"proposal"},"state_volume":{"enabled":false},
        "network":{"profile":"disabled"},"triggers":{"push":true,"refs":["refs/heads/main"]}
    });
    // Exercise the actual typed configuration consumed by the Local workspace
    // adapter. This is declared parent metadata, not a builder execution proof.
    let typed: agent_config::AgentConfig =
        serde_json::from_value(value).expect("complete typed release configuration");
    serde_json::to_value(typed).unwrap()
}
