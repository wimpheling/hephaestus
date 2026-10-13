//! Typed binding construction through the declared app RLS boundary.

use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use capability_domain::{
    CapabilityBinding, CapabilityBindingId, CapabilityOperation, CapabilityRequirement,
    CapabilityRequirementId, CapabilityResource, CapabilityResourceKind, CapabilitySlotKey,
};

pub async fn establish(birth: &crate::authority_bootstrap::Bootstrap) {
    let project = birth.fixture.consuming_project;
    let issuer = birth.fixture.outsider;
    let mut tx = begin_actor_transaction(&birth.app, &birth.actor)
        .await
        .unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(issuer.as_uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO project_capability_granters(project_id,user_id,created_by) VALUES($1,$2,$3)",
    )
    .bind(project)
    .bind(issuer.as_uuid())
    .bind(birth.actor.user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    // A distinct real seeded user receives live rights through the actor boundary.
    // No saved invoking identity is borrowed to grant workload capabilities.
    let identity = crate::support::identity(issuer);
    let mut tx = begin_actor_transaction(&birth.app, &identity)
        .await
        .unwrap();
    let actor: (String, String) = sqlx::query_as("SELECT current_user::text,hephaestus_actor_id()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(actor, ("hephaestus_app".to_owned(), issuer.to_string()));
    let rights: (bool, bool) = sqlx::query_as("SELECT check_permission('user',$1::text,'can_manage','agent_instance',$2::text)=1,can_grant_agent_capability_operations($1,'project',$3,ARRAY['inspect'])")
        .bind(issuer.as_uuid()).bind(birth.input.instance_id().as_uuid()).bind(project)
        .fetch_one(&mut *tx).await.unwrap();
    assert_eq!(rights, (true, true));
    // Read immutable published comparison data through the existing worker
    // SELECT port. Consuming-project grant authority does not imply source
    // catalogue read rights. The binding writes below remain this issuer's app
    // transaction, with current RLS and the declared requirement/hash trigger.
    let requirements: Vec<(uuid::Uuid,String)> = sqlx::query_as("SELECT id,slot_key FROM release_capability_requirements WHERE release_agent_id=$1 ORDER BY slot_key")
        .bind(birth.fixture.release_agent).fetch_all(&birth.worker).await.unwrap();
    assert_eq!(requirements.len(), 2);
    for (id, slot) in requirements {
        let requirement = CapabilityRequirement::new(
            CapabilityRequirementId::from_uuid(id),
            CapabilitySlotKey::parse(slot).unwrap(),
            CapabilityResourceKind::Project,
            [],
            [CapabilityOperation::Inspect],
            false,
        )
        .unwrap();
        let binding = CapabilityBinding::bind(
            CapabilityBindingId::new(),
            &requirement,
            CapabilityResource::new(CapabilityResourceKind::Project, project),
            [CapabilityOperation::Inspect],
        )
        .unwrap();
        sqlx::query("INSERT INTO agent_capability_bindings(id,instance_revision_id,release_agent_id,requirement_id,requirement_hash,slot_key,resource_kind,resource_id,granted_operations,normalized_hash,authorization_model_version,created_by) VALUES($1,$2,$3,$4,$5,$6,'project',$7,ARRAY['inspect'],$8,$9,$10)")
            .bind(binding.id().as_uuid()).bind(birth.input.revision_id().as_uuid())
            .bind(birth.fixture.release_agent).bind(id)
            .bind(requirement.normalized_hash().as_bytes().as_slice()).bind(binding.slot().as_str())
            .bind(project).bind(binding.normalized_hash().as_bytes().as_slice())
            .bind(AUTHORIZATION_MODEL_VERSION).bind(issuer.as_uuid())
            .execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn genuine_optional_binding_issuer_delegation_and_operation_loss_deny(pool: sqlx::PgPool) {
    let f = Box::pin(super::fixture::Fixture::new(&pool, 2)).await;
    let issuer = f.birth.birth.fixture.outsider.as_uuid();
    let project = f.birth.birth.fixture.consuming_project;
    let bound:(i64,bool)=sqlx::query_as("SELECT count(*),bool_and(created_by=$2 AND authorization_model_version=$3) FROM agent_capability_bindings WHERE instance_revision_id=$1")
        .bind(f.run.instance_revision_id.as_uuid()).bind(issuer).bind(AUTHORIZATION_MODEL_VERSION)
        .fetch_one(&pool).await.unwrap();
    assert_eq!(bound, (2, true));
    f.allowed().await;
    // Remove only issuer delegation; current invoker rights stay intact.
    sqlx::query("DELETE FROM project_capability_granters WHERE project_id=$1 AND user_id=$2")
        .bind(project)
        .bind(issuer)
        .execute(&pool)
        .await
        .unwrap();
    let grant: bool = sqlx::query_scalar(
        "SELECT can_grant_agent_capability_operations($1,'project',$2,ARRAY['inspect'])",
    )
    .bind(issuer)
    .bind(project)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!grant);
    assert_eq!(super::rights::caller_rights(&f).await, (true, true));
    f.denied(&f.run).await;
    let mut tx = begin_actor_transaction(&f.birth.birth.app, &f.birth.birth.actor)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO project_capability_granters(project_id,user_id,created_by) VALUES($1,$2,$3)",
    )
    .bind(project)
    .bind(issuer)
    .bind(f.birth.birth.actor.user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    f.allowed().await;
    // The issuer is not an organization member. Its Inspect permission comes
    // from this exact maintainership; keep the explicit delegation present.
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(project)
        .bind(issuer)
        .execute(&pool)
        .await
        .unwrap();
    let operations:(bool,bool,bool,bool)=sqlx::query_as("SELECT check_permission('user',$1::text,'can_grant_agent_capability','project',$2::text)=1,check_permission('user',$1::text,'can_read','project',$2::text)=1,can_grant_agent_capability_operations($1,'project',$2,ARRAY['inspect']),check_permission('agent_instance',$3::text,'agent_inspect','project',$2::text)=1")
        .bind(issuer).bind(project).bind(f.run.instance_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(operations, (true, false, false, true));
    assert_eq!(super::rights::caller_rights(&f).await, (true, true));
    f.denied(&f.run).await;
}
