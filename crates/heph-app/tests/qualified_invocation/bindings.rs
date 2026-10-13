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
