use super::super::ledger::Ledger;
use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use recipe_application::{EffectClaim, PlannedResourceIdentity, ResourceAction};
use release_domain::ContentHash;
use serde::Serialize;
use uuid::Uuid;
use volume_trait::{VolumeCreationIdentityWire, VolumeCreationOperationId, VolumeOwnershipScopeId};

// Exact frozen 106 ClaimWire field order: this fixture changes UUID labels only,
// not the canonical digest. SQL checks equality against the actual persisted JSON.
#[derive(Serialize)]
struct ClaimWire {
    command_id: Uuid,
    deployment_id: Uuid,
    resource: String,
    identity: serde_json::Value,
    input_hash: ContentHash,
    resource_version: u64,
    generation: u64,
    action: ResourceAction,
    attempt_id: Uuid,
    actor_id: Uuid,
    request_id: Uuid,
}
fn bytes(claim: &EffectClaim) -> Vec<u8> {
    serde_json::to_vec(&ClaimWire {
        command_id: claim.command.id().as_uuid(),
        deployment_id: claim.deployment_id.as_uuid(),
        resource: claim.resource.as_str().into(),
        identity: serde_json::to_value(claim.identity).unwrap(),
        input_hash: claim.input_hash,
        resource_version: claim.resource_version,
        generation: claim.generation,
        action: claim.action,
        attempt_id: claim.provenance.attempt_id.as_uuid(),
        actor_id: claim.provenance.actor_id.as_uuid(),
        request_id: claim.provenance.request_id.as_uuid(),
    })
    .unwrap()
}
pub async fn insert_nonderived_labels(ledger: &Ledger) {
    let claim = &ledger.target;
    let PlannedResourceIdentity::Volume {
        id,
        filesystem_uuid: Some(filesystem_uuid),
    } = claim.identity
    else {
        panic!("volume");
    };
    let canonical = bytes(claim);
    let digest = ContentHash::digest(&canonical);
    let scope = Uuid::new_v5(
        &claim.deployment_id.as_uuid(),
        b"hephaestus-volume-owner-scope-v1\0",
    );
    let effect = Uuid::new_v4();
    let operation = Uuid::new_v4();
    let creation = VolumeCreationIdentityWire {
        scope_id: VolumeOwnershipScopeId::from_uuid(scope).unwrap(),
        operation_id: VolumeCreationOperationId::from_uuid(operation).unwrap(),
        command_id: claim.command.id().as_uuid(),
        effect_id: effect,
        attempt_id: claim.provenance.attempt_id.as_uuid(),
        actor_id: claim.provenance.actor_id.as_uuid(),
        request_id: claim.provenance.request_id.as_uuid(),
        resource_version: claim.resource_version,
        generation: claim.generation,
        input_hash: *claim.input_hash.as_bytes(),
        intent_hash: *ledger.intent.input_hash().as_bytes(),
    };
    let mut tx = begin_actor_transaction(&ledger.app, &ledger.identity)
        .await
        .unwrap();
    let object = ObjectRef::new(ObjectType::Project, ledger.seeded.consuming_project);
    let decision = PostgresMelangeAuthorizer
        .check(
            &mut tx,
            Subject::User(ledger.identity.user_id),
            Permission::CanManage,
            object,
        )
        .await
        .unwrap();
    assert!(decision.is_allowed());
    audit_decision(
        &mut tx,
        ledger.identity.user_id,
        Permission::CanManage,
        object,
        decision,
        ledger.identity.request_id,
    )
    .await
    .unwrap();
    let occurrence = Uuid::new_v4();
    sqlx::query("SELECT set_config('hephaestus.occurrence_id',$1,true)")
        .bind(occurrence.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let(event,cursor,version):(Uuid,i64,i64)=sqlx::query_as("SELECT event_id,cursor,aggregate_version FROM append_application_event($1,'project',$2,'project',$2,'project.changed','updated',NULL,$3,NULL)").bind(occurrence).bind(ledger.seeded.consuming_project).bind(ledger.seeded.organization).fetch_one(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO recipe_owned_volume_preparations(attempt_id,deployment_id,project_id,resource_name,mapping_version,canonical_claim,claim_digest,scope_id,effect_id,registration_operation_id,creation_json,volume_id,capacity_bytes,filesystem_uuid,actor_id,request_id,event_id,event_cursor,event_aggregate_version) VALUES($1,$2,$3,$4,1,$5,$6,$7,$8,$9,$10,$11,16777216,$12,$13,$14,$15,$16,$17)")
        .bind(claim.provenance.attempt_id.as_uuid()).bind(claim.deployment_id.as_uuid()).bind(ledger.seeded.consuming_project).bind(claim.resource.as_str())
        .bind(canonical).bind(digest.as_bytes().as_slice()).bind(scope).bind(effect).bind(operation).bind(serde_json::to_value(creation).unwrap())
        .bind(id.as_uuid()).bind(filesystem_uuid).bind(ledger.identity.user_id.as_uuid()).bind(ledger.identity.request_id.as_uuid()).bind(event).bind(cursor).bind(version)
        .execute(&mut *tx).await.expect("SQL validates provenance shape, not Rust UUIDv5 derivation");
    tx.commit().await.unwrap();
}
