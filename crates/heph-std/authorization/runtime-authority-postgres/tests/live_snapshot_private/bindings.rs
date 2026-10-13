use capability_domain::{
    CapabilityBinding, CapabilityBindingId, CapabilityOperation, CapabilityRequirement,
    CapabilityRequirementId, CapabilityResource, CapabilityResourceKind, CapabilitySlotKey,
};
use runtime_authority::LiveRunSnapshotRequest;
use sqlx::PgPool;
use uuid::Uuid;

use super::{fixture, sessions};
use crate::support::Fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn appended_optional_binding_cannot_expand_an_already_issued_ceiling(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let (catalog, requirement) = optional_catalog(&pool, &f).await;
    let e = fixture::normal(&pool, &catalog, "owned_canonical").await;
    let repository = fixture::repository(&pool).await;
    let snapshot = sessions::snapshot(&repository, &e).await;
    assert_eq!(snapshot.bindings().len(), 0);
    fixture::advance(&pool, e.run, "provisioning").await;
    let session = sessions::issue(&repository, &e, &snapshot).await;
    let request = sessions::request(&e, &session, &snapshot);
    insert_binding(&pool, &catalog, e.revision, &requirement).await;
    // The new authorized ceiling is real and non-empty, but cannot replace the
    // earlier immutable session ceiling. Nothing mutates published requirements.
    let fresh = sessions::snapshot(&repository, &e).await;
    assert_eq!(fresh.bindings().len(), 1);
    assert_ne!(fresh.normalized_hash(), snapshot.normalized_hash());
    fixture::advance(&pool, e.run, "running").await;
    fixture::denied(&repository, &request).await;
    fixture::denied(
        &repository,
        &LiveRunSnapshotRequest::pre_issuance(fixture::pins(&e)),
    )
    .await;
}

async fn optional_catalog(pool: &PgPool, original: &Fixture) -> (Fixture, CapabilityRequirement) {
    let mut f = *original;
    let release = Uuid::new_v4();
    let agent = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let requirement = CapabilityRequirement::new(
        CapabilityRequirementId::new(),
        CapabilitySlotKey::parse("source").unwrap(),
        CapabilityResourceKind::Project,
        [],
        [CapabilityOperation::Inspect],
        false,
    )
    .unwrap();
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state) SELECT $1,repository_id,$1::text,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'draft' FROM releases WHERE id=$2")
        .bind(release).bind(f.release).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) SELECT $1,$2,family_id,$1::text,display_name,runtime_contract,runtime_contract_hash,requires_state FROM release_agents WHERE id=$3")
        .bind(agent).bind(release).bind(f.release_agent).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO release_capability_requirements(id,release_agent_id,slot_key,purpose,resource_kind,required_operations,optional_operations,slot_required,normalized_hash) VALUES($1,$2,'source','Inspect optional project','project','{}',ARRAY['inspect'],false,$3)")
        .bind(requirement.id().as_uuid()).bind(agent).bind(requirement.normalized_hash().as_bytes().as_slice()).execute(pool).await.unwrap();
    sqlx::query("UPDATE releases SET state='published',published_at=now() WHERE id=$1")
        .bind(release)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,instance_id,$2,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,true FROM agent_instance_revisions WHERE id=(SELECT active_revision_id FROM agent_instances WHERE id=$3)")
        .bind(revision).bind(agent).bind(f.instance).execute(pool).await.unwrap();
    sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
        .bind(f.instance)
        .bind(revision)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO project_capability_granters(project_id,user_id,created_by) VALUES($1,$2,$2)",
    )
    .bind(f.consuming_project)
    .bind(f.maintainer.as_uuid())
    .execute(pool)
    .await
    .unwrap();
    f.release = release;
    f.release_agent = agent;
    (f, requirement)
}

async fn insert_binding(
    pool: &PgPool,
    f: &Fixture,
    revision: Uuid,
    requirement: &CapabilityRequirement,
) {
    let binding = CapabilityBinding::bind(
        CapabilityBindingId::new(),
        requirement,
        CapabilityResource::new(CapabilityResourceKind::Project, f.consuming_project),
        [CapabilityOperation::Inspect],
    )
    .unwrap();
    sqlx::query("INSERT INTO agent_capability_bindings(id,instance_revision_id,release_agent_id,requirement_id,requirement_hash,slot_key,resource_kind,resource_id,granted_operations,normalized_hash,authorization_model_version,created_by) VALUES($1,$2,$3,$4,$5,'source','project',$6,ARRAY['inspect'],$7,'live-fixture/v1',$8)")
        .bind(binding.id().as_uuid()).bind(revision).bind(f.release_agent).bind(requirement.id().as_uuid()).bind(requirement.normalized_hash().as_bytes().as_slice()).bind(f.consuming_project).bind(binding.normalized_hash().as_bytes().as_slice()).bind(f.maintainer.as_uuid()).execute(pool).await.unwrap();
}
