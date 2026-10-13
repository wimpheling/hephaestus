use crate::{ReleaseService, ReleaseServiceError, support};
use release_domain::{
    AgentInstanceId, AgentInstanceRevisionId, InstanceName, ReleaseAgentId, ReleaseCommandKey,
    ReleaseId,
};
use release_service::ImportAgentWithVolumes;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn requirements(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
    template: &ImportAgentWithVolumes,
) {
    for secret in [true, false] {
        let agent = ReleaseAgentId::new();
        let release = ReleaseId::new();
        sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state) SELECT $1,repository_id,$1::text,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'draft' FROM releases WHERE id=$2")
            .bind(release.as_uuid()).bind(template.release_id.as_uuid()).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state,parameter_schema,secret_slot_schema) SELECT $1,$5,family_id,$3,'Unsupported',runtime_contract,runtime_contract_hash,requires_state,parameter_schema,$4 FROM release_agents WHERE id=$2")
            .bind(agent.as_uuid()).bind(template.release_agent_id.as_uuid()).bind(if secret {"required-secret"} else {"required-capability"}).bind(if secret {json!([{"key":"token","required":true}])} else {json!([])}).bind(release.as_uuid()).execute(pool).await.unwrap();
        if !secret {
            sqlx::query("INSERT INTO release_capability_requirements(id,release_agent_id,slot_key,purpose,resource_kind,required_operations,optional_operations,slot_required,normalized_hash) VALUES(gen_random_uuid(),$1,'source','Unsupported generic slot','project',ARRAY['inspect'],'{}',true,$2)")
                .bind(agent.as_uuid()).bind([23_u8;32].as_slice()).execute(pool).await.unwrap();
        }
        sqlx::query("UPDATE releases SET state='published',published_at=now() WHERE id=$1 AND state='draft'")
            .bind(release.as_uuid()).execute(pool).await.unwrap();
        let mut command = template.clone();
        command.release_agent_id = agent;
        command.release_id = release;
        assert!(matches!(
            service
                .import_agent_with_volumes(&support::identity(seed.maintainer), command.clone())
                .await,
            Err(ReleaseServiceError::CapabilityResourceUnavailable)
        ));
        absent(pool, &command).await;
    }
}

pub async fn grant_conflict_rolls_back(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
    committed: &ImportAgentWithVolumes,
) {
    let mut command = committed.clone();
    command.command_key =
        ReleaseCommandKey::derive("atomic-conflict", &[Uuid::new_v4().as_bytes()]);
    command.instance_id = AgentInstanceId::new();
    command.revision_id = AgentInstanceRevisionId::new();
    command.name = InstanceName::parse("grant-conflict").unwrap();
    assert!(
        service
            .import_agent_with_volumes(&support::identity(seed.maintainer), command.clone())
            .await
            .is_err(),
        "a grant ID belonging to another scope cannot be reused"
    );
    absent(pool, &command).await;
    let revisions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_instance_revisions WHERE id=$1")
            .bind(command.revision_id.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(revisions, 0);
    let commands: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_command_inbox WHERE command_key=$1")
            .bind(command.command_key.as_bytes().as_slice())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(commands, 0);
}

async fn absent(pool: &PgPool, command: &ImportAgentWithVolumes) {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_instances WHERE id=$1")
        .bind(command.instance_id.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

pub async fn inbox_forgery(
    pool: &PgPool,
    seed: &support::Fixture,
    command: &ImportAgentWithVolumes,
) {
    let actors = crate::roles::role_pool(pool, crate::roles::Role::Actor).await;
    let actor = support::identity(seed.maintainer);
    let old_revision: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id=$1")
            .bind(seed.instance)
            .fetch_one(pool)
            .await
            .unwrap();
    for (operation, creator, secondary) in [
        (
            "import_agent_with_volumes",
            seed.member.as_uuid(),
            command.revision_id.as_uuid(),
        ),
        (
            "import_agent_with_volumes",
            seed.maintainer.as_uuid(),
            old_revision,
        ),
        (
            "request_instance_removal",
            seed.maintainer.as_uuid(),
            Uuid::new_v4(),
        ),
    ] {
        let mut tx = authz_postgres::begin_actor_transaction(&actors, &actor)
            .await
            .unwrap();
        let key = ReleaseCommandKey::derive("forged-inbox", &[Uuid::new_v4().as_bytes()]);
        let error=sqlx::query("INSERT INTO release_command_inbox(command_key,operation,aggregate_id,secondary_id,actor_id,request_id,input_hash) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(key.as_bytes().as_slice()).bind(operation).bind(command.instance_id.as_uuid()).bind(secondary).bind(creator).bind(actor.request_id.as_uuid()).bind([41_u8;32].as_slice()).execute(&mut *tx).await.expect_err("actor role cannot forge mismatched import/removal evidence");
        assert_eq!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("42501")
        );
        tx.rollback().await.unwrap();
    }
    actors.close().await;
    event_forgery(pool, seed, command, old_revision).await;
}

async fn event_forgery(
    pool: &PgPool,
    seed: &support::Fixture,
    command: &ImportAgentWithVolumes,
    old_revision: Uuid,
) {
    let actors = crate::roles::role_pool(pool, crate::roles::Role::Actor).await;
    let mut actor = support::identity(seed.maintainer);
    let original_request: Uuid =
        sqlx::query_scalar("SELECT request_id FROM release_command_inbox WHERE command_key=$1")
            .bind(command.command_key.as_bytes().as_slice())
            .fetch_one(pool)
            .await
            .unwrap();
    actor.request_id = identity_domain::RequestId::from_uuid(original_request);
    let payload = json!({"runnable":true,"volume_mode":"named","dispatch_supported":false});
    for (creator, revision, event_type, body) in [
        (
            seed.member.as_uuid(),
            command.revision_id.as_uuid(),
            "instance.created",
            payload.clone(),
        ),
        (
            seed.maintainer.as_uuid(),
            old_revision,
            "instance.created",
            payload.clone(),
        ),
        (
            seed.maintainer.as_uuid(),
            command.revision_id.as_uuid(),
            "instance.removed",
            payload,
        ),
        (
            seed.maintainer.as_uuid(),
            command.revision_id.as_uuid(),
            "instance.created",
            json!({"runnable":true,"volume_mode":"named","dispatch_supported":true}),
        ),
    ] {
        let mut tx = authz_postgres::begin_actor_transaction(&actors, &actor)
            .await
            .unwrap();
        let error=sqlx::query("INSERT INTO agent_instance_events(id,instance_id,revision_id,event_type,actor_id,request_id,payload) VALUES(gen_random_uuid(),$1,$2,$3,$4,$5,$6)")
            .bind(command.instance_id.as_uuid()).bind(revision).bind(event_type).bind(creator).bind(actor.request_id.as_uuid()).bind(body).execute(&mut *tx).await.expect_err("actor event must match exact committed typed operation and payload");
        assert_eq!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("42501")
        );
        tx.rollback().await.unwrap();
    }
    actors.close().await;
}
