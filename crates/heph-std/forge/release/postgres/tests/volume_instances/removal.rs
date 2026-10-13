use crate::{ReleaseService, ReleaseServiceError, history, removal_command, support};
use release_domain::AgentInstanceId;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn exercise(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
    lease: Uuid,
) {
    let denied = support::identity(seed.member);
    let version: i64 = sqlx::query_scalar("SELECT version FROM agent_instances WHERE id=$1")
        .bind(seed.instance)
        .fetch_one(pool)
        .await
        .unwrap();
    let command = removal_command(seed.instance, u64::try_from(version).unwrap());
    assert!(matches!(
        service
            .request_instance_removal(&denied, command.clone())
            .await,
        Err(ReleaseServiceError::AuthorizationDenied)
    ));
    // Cleanup admission remains available to a current manager after the
    // original creator loses source authority; it never requires source CanUse.
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(seed.project)
        .bind(seed.maintainer.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    let source: i32 =
        sqlx::query_scalar("SELECT check_permission('user',$1,'can_use','release_agent',$2)")
            .bind(seed.maintainer.to_string())
            .bind(seed.release_agent.to_string())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(source, 0);
    let actor = support::identity(seed.maintainer);
    let before_lease = history::lease_snapshot(pool, lease).await;
    let before_volume = history::volume_snapshot(pool, seed.volume).await;
    let record = crate::race::grant_then_close(pool, seed, service, &actor, command.clone()).await;
    assert_eq!(record.instance_id, command.instance_id);
    assert_eq!(
        service
            .request_instance_removal(&actor, command.clone())
            .await
            .unwrap(),
        record
    );
    assert_eq!(
        history::lease_snapshot(pool, lease).await,
        before_lease,
        "no attachment fence released by actor admission"
    );
    assert_eq!(
        history::volume_snapshot(pool, seed.volume).await,
        before_volume,
        "no physical resource transition claimed"
    );
    assert_cancellation(pool, seed, service, &actor, &command).await;
    denied_fresh_grant(pool, seed, service, &actor).await;
    guards(pool, seed).await;
    replay_and_role_restore(pool, seed, service, &actor, command).await;
}

async fn assert_cancellation(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
    actor: &identity_domain::AuthenticatedIdentity,
    command: &release_service::RequestInstanceRemoval,
) {
    let status = service
        .instance_volume_status(actor, AgentInstanceId::from_uuid(seed.instance))
        .await
        .unwrap();
    assert!(!status.run_gate_open);
    assert_eq!(status.removal_id, Some(command.removal_id));
    let (state, cancel): (String, bool) =
        sqlx::query_as("SELECT state,cancel_requested_at IS NOT NULL FROM runs WHERE id=$1")
            .bind(seed.run)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(state, "leasing_volume");
    assert!(cancel);
    let messages:i64=sqlx::query_scalar("SELECT count(*) FROM outbox WHERE aggregate_id=$1 AND subject='heph.run.command.cancel.v1'").bind(seed.run).fetch_one(pool).await.unwrap();
    assert_eq!(messages, 1, "retry creates no second cancellation effect");
    let audit:i64=sqlx::query_scalar("SELECT count(*) FROM authorization_audit_events WHERE request_id=$1 AND permission='can_use'").bind(actor.request_id.as_uuid()).fetch_one(pool).await.unwrap();
    assert_eq!(audit, 0, "removal does not require source access");
}

async fn replay_and_role_restore(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
    actor: &identity_domain::AuthenticatedIdentity,
    command: release_service::RequestInstanceRemoval,
) {
    let mut changed = command.clone();
    changed.expected_version += 1;
    assert!(matches!(
        service.request_instance_removal(actor, changed).await,
        Err(ReleaseServiceError::IdempotencyConflict)
    ));
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(seed.consuming_project)
        .bind(seed.maintainer.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    assert!(
        matches!(
            service.request_instance_removal(actor, command).await,
            Err(ReleaseServiceError::AuthorizationDenied)
        ),
        "current management checked before replay"
    );
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2),($3,$2)")
        .bind(seed.project)
        .bind(seed.maintainer.as_uuid())
        .bind(seed.consuming_project)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE agent_instances SET run_gate_open=true WHERE id=$1")
            .bind(seed.instance)
            .execute(pool)
            .await
            .is_err(),
        "role restoration cannot reopen permanent closure"
    );
}

async fn guards(pool: &PgPool, seed: &support::Fixture) {
    assert!(
        sqlx::query("UPDATE agent_instances SET run_gate_open=true WHERE id=$1")
            .bind(seed.instance)
            .execute(pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE agent_instances SET state='removed',removed_at=now() WHERE id=$1")
            .bind(seed.instance)
            .execute(pool)
            .await
            .is_err(),
        "no tombstone without actual worker cleanup proof"
    );
    assert!(
        sqlx::query("UPDATE runs SET state='provisioning' WHERE id=$1")
            .bind(seed.run)
            .execute(pool)
            .await
            .is_err(),
        "queued run cannot acquire after permanent close"
    );
    assert!(sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES(gen_random_uuid(),$1,$2,$3,'host',20,'active',now()+interval '1 hour')").bind(seed.volume).bind(seed.instance).bind(seed.run).execute(pool).await.is_err(),"new lease admission rejected");
    assert!(
        sqlx::query("DELETE FROM agent_instance_removal_requests WHERE instance_id=$1")
            .bind(seed.instance)
            .execute(pool)
            .await
            .is_err(),
        "closure retained"
    );
    assert!(sqlx::query("UPDATE agent_instances SET volume_mode='named',run_gate_open=false,state_volume_id=NULL WHERE id=$1").bind(seed.instance).execute(pool).await.is_err(),"mode remains immutable");
}

pub async fn named(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
    command: &release_service::ImportAgentWithVolumes,
) {
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(seed.consuming_project)
        .bind(seed.owner.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(seed.consuming_project)
        .bind(seed.maintainer.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    let actor = support::identity(seed.owner);
    let before = service
        .instance_volume_status(&actor, command.instance_id)
        .await
        .unwrap();
    assert!(!before.dispatch_supported);
    assert!(!before.run_gate_open);
    assert!(before.removal_id.is_none());
    let admission = service
        .request_instance_removal(&actor, removal_command(command.instance_id.as_uuid(), 1))
        .await
        .unwrap();
    let counts:(i64,i64)=sqlx::query_as("SELECT count(*),count(revoked.id) FROM agent_instance_volume_mount_grants mount_grant LEFT JOIN agent_instance_volume_mount_revocations revoked ON revoked.grant_id=mount_grant.id WHERE mount_grant.instance_id=$1").bind(command.instance_id.as_uuid()).fetch_one(pool).await.unwrap();
    assert_eq!(counts, (2, 2));
    let after = service
        .instance_volume_status(&actor, command.instance_id)
        .await
        .unwrap();
    assert_eq!(after.removal_id, Some(admission.removal_id));
    let retained:i64=sqlx::query_scalar("SELECT count(*) FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=$1").bind(command.revision_id.as_uuid()).fetch_one(pool).await.unwrap();
    assert_eq!(retained, 2);
    let state: String = sqlx::query_scalar("SELECT state FROM agent_instances WHERE id=$1")
        .bind(command.instance_id.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(state, "active", "admission does not claim terminal cleanup");
    assert!(sqlx::query("UPDATE release_command_inbox SET input_hash=$2 WHERE aggregate_id=$1 AND operation='import_agent_with_volumes'").bind(command.instance_id.as_uuid()).bind([77_u8;32].as_slice()).execute(pool).await.is_err(),"exact input evidence immutable");
    assert!(sqlx::query("INSERT INTO agent_instance_volume_mount_grants(id,instance_revision_id,instance_id,release_agent_id,slot_key,volume_id,access_mode,release_contract_hash,created_by,request_id,authorization_model_version) SELECT gen_random_uuid(),instance_revision_id,instance_id,release_agent_id,slot_key,volume_id,access_mode,release_contract_hash,created_by,request_id,authorization_model_version FROM agent_instance_volume_mount_grants WHERE instance_id=$1 LIMIT 1").bind(command.instance_id.as_uuid()).execute(pool).await.is_err(),"closed consumer cannot acquire new authority");
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(seed.consuming_project)
        .bind(seed.maintainer.as_uuid())
        .execute(pool)
        .await
        .unwrap();
}

async fn denied_fresh_grant(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
    actor: &identity_domain::AuthenticatedIdentity,
) {
    use capability_domain::{AuthorityHash, CapabilitySlotKey};
    use runtime_types::{AgentInstanceRevisionId, ReleaseAgentId, VolumeId};
    use volume_domain::{VolumeAccessMode, VolumeMountGrantId, VolumeMountScope};
    use volume_trait::VolumeMountGrantRepository;
    let revision: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id=$1")
            .bind(seed.instance)
            .fetch_one(pool)
            .await
            .unwrap();
    let scope = VolumeMountScope::new(
        AgentInstanceId::from_uuid(seed.instance),
        AgentInstanceRevisionId::from_uuid(revision),
        ReleaseAgentId::from_uuid(seed.release_agent),
        CapabilitySlotKey::parse("state").unwrap(),
        VolumeId::from_uuid(seed.volume),
        VolumeAccessMode::ReadWrite,
        AuthorityHash::from_bytes([4; 32]),
    )
    .unwrap();
    let source: bool = sqlx::query_scalar("SELECT private_volume_mount_source_is_live($1,$2)")
        .bind(actor.user_id.as_uuid())
        .bind(seed.volume)
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(source);
    assert!(
        service
            .grant_volume_mount(actor, VolumeMountGrantId::new(), &scope)
            .await
            .is_err(),
        "even a current authorized source manager cannot grant a closed revision"
    );
}
