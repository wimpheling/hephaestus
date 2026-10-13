use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use heph_run::{Run, RunAuthorityManager, RunKind, RunOutcome, RunState};
use release_service::InstanceExecutionConfiguration;
use run_orchestrator::RunLaunchAuthorizer;
use runtime_authority::RuntimeSessionRepository;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;
use volume_trait::RunVolumeStore;

use super::{composition, fixture::Fixture};

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_checks_whole_persisted_run_before_kind_or_any_authority_operation(pool: PgPool) {
    let f = Box::pin(Fixture::provisioning(&pool, 0)).await;
    let run = f.run().await;
    let mut contradictory: Vec<(&str, Run)> = Vec::new();
    macro_rules! changed {
        ($field:ident, $value:expr) => {{
            let mut supplied = run.clone();
            supplied.$field = $value;
            contradictory.push((stringify!($field), supplied));
        }};
    }
    changed!(id, runtime_types::RunId::new());
    changed!(instance_id, runtime_types::AgentInstanceId::new());
    changed!(
        instance_revision_id,
        runtime_types::AgentInstanceRevisionId::new()
    );
    changed!(release_id, release_domain::ReleaseId::new());
    changed!(release_agent_id, release_domain::ReleaseAgentId::new());
    changed!(attachment_id, Some(runtime_types::AgentAttachmentId::new()));
    changed!(kind, RunKind::Normal);
    changed!(requires_state, !run.requires_state);
    changed!(command_id, runtime_types::CommandId::new());
    changed!(volume_id, Some(runtime_types::VolumeId::new()));
    changed!(lease_id, Some(runtime_types::LeaseId::new()));
    changed!(lease_fencing_token, Some(1));
    changed!(vm_id, Some("changed-vm".to_owned()));
    changed!(state, RunState::LeasingVolume);
    changed!(outcome, Some(RunOutcome::Succeeded));
    changed!(
        exit,
        Some(heph_runtime::VmExit {
            code: Some(0),
            signal: None
        })
    );
    changed!(failure, Some("changed diagnostic".to_owned()));
    changed!(cancel_requested_at, Some(super::fixture::pg_time()));
    changed!(created_at, run.created_at + time::Duration::microseconds(1));
    changed!(updated_at, run.updated_at + time::Duration::microseconds(1));
    changed!(state_version, run.state_version + 1);
    assert_eq!(contradictory.len(), 21);
    for (field, supplied) in contradictory {
        assert_ne!(supplied, run, "{field} contradiction is genuine");
        f.denied_unchanged(&supplied).await;
    }
    f.issue().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_checked_configuration_and_worker_role_are_required_for_empty_set(pool: PgPool) {
    let f = Box::pin(Fixture::provisioning(&pool, 0)).await;
    let run = f.run().await;
    let config = &f.birth.config;
    let changed = InstanceExecutionConfiguration::new(
        config.provider_scope().clone(),
        config.volume_root().to_owned(),
        config.volume_namespace(),
        config.platform_policy().clone(),
        "changed/server-platform".to_owned(),
    )
    .unwrap();
    let before = f.rows().await;
    for (role, cfg) in [
        (f.birth.birth.worker.clone(), changed),
        (f.birth.birth.app.clone(), config.clone()),
        (pool.clone(), config.clone()),
    ] {
        let manager = composition::manager(&f, role.clone(), cfg.clone(), Duration::from_secs(60));
        assert!(manager.prepare(&run).await.is_err());
        assert!(manager.reauthorize(&run).await.is_err());
        assert!(
            manager
                .acknowledge(&run, run.id.as_uuid(), 1)
                .await
                .is_err()
        );
        let launcher = composition::launcher(&f, role, cfg);
        assert!(launcher.authorize(&run).await.is_err());
        assert!(launcher.authorize_with_volumes(&run).await.is_err());
        assert_eq!(f.rows().await, before);
        assert!(f.handoff_files().is_empty());
    }
    f.issue().await;
}

async fn withdraw_source(f: &Fixture) {
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.birth.birth.fixture.project)
        .bind(f.birth.birth.actor.user_id.as_uuid())
        .execute(&f.birth.pool)
        .await
        .unwrap();
    let allowed: bool = sqlx::query_scalar(
        "SELECT check_permission('user',$1::text,'can_use','release_agent',$2::text)=1",
    )
    .bind(f.birth.birth.actor.user_id.as_uuid())
    .bind(f.birth.start.release_agent_id.as_uuid())
    .fetch_one(&f.birth.pool)
    .await
    .unwrap();
    assert!(
        !allowed,
        "current saved-invoker release-use permission was removed"
    );
}
async fn withdraw_issuer(f: &Fixture) {
    let issuer = f.birth.birth.fixture.outsider.as_uuid();
    let project = f.birth.birth.fixture.consuming_project;
    sqlx::query("DELETE FROM project_capability_granters WHERE project_id=$1 AND user_id=$2")
        .bind(project)
        .bind(issuer)
        .execute(&f.birth.pool)
        .await
        .unwrap();
    let rights: (bool, bool, bool) = sqlx::query_as("SELECT check_permission('user',$1::text,'can_execute','agent_instance',$2::text)=1,check_permission('user',$1::text,'can_use','release_agent',$3::text)=1,can_grant_agent_capability_operations($4,'project',$5,ARRAY['inspect'])")
        .bind(f.birth.birth.actor.user_id.as_uuid()).bind(f.birth.start.instance_id.as_uuid())
        .bind(f.birth.start.release_agent_id.as_uuid()).bind(issuer).bind(project)
        .fetch_one(&f.birth.pool).await.unwrap();
    assert_eq!(
        rights,
        (true, true, false),
        "only distinct binding issuer delegation withdrawn"
    );
}
async fn revoke_second_mount(f: &Fixture) {
    let run = f.run().await;
    let selections = f.volumes.load_run_selections(run.id).await.unwrap();
    assert_eq!(selections.selections().len(), 2);
    let volume = selections.selections()[1].scope().volume_id().as_uuid();
    let actor = &f.birth.birth.actor;
    let mut tx = begin_actor_transaction(&f.birth.birth.app, actor)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_instance_volume_mount_revocations(id,grant_id,created_by,request_id,authorization_model_version) SELECT $1,id,$2,$4,$5 FROM agent_instance_volume_mount_grants WHERE instance_revision_id=$3 AND volume_id=$6")
        .bind(Uuid::new_v4()).bind(actor.user_id.as_uuid()).bind(run.instance_revision_id.as_uuid())
        .bind(actor.request_id.as_uuid()).bind(AUTHORIZATION_MODEL_VERSION).bind(volume)
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_instance_volume_mount_grants g WHERE instance_revision_id=$1 AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations WHERE grant_id=g.id)")
        .bind(run.instance_revision_id.as_uuid()).fetch_one(&f.birth.pool).await.unwrap();
    assert_eq!(
        remaining, 1,
        "one remaining grant cannot authorize the whole two-set"
    );
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_source_withdrawal_before_first_issue_cannot_create_handoff(pool: PgPool) {
    let f = Box::pin(Fixture::provisioning(&pool, 0)).await;
    withdraw_source(&f).await;
    f.denied_unchanged(&f.run().await).await;
    assert!(f.handoff_files().is_empty());
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_distinct_binding_issuer_withdrawal_before_issue_denies_whole_ceiling(pool: PgPool) {
    let f = Box::pin(Fixture::provisioning(&pool, 2)).await;
    withdraw_issuer(&f).await;
    f.denied_unchanged(&f.run().await).await;
    assert!(f.handoff_files().is_empty());
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_second_mount_revocation_before_issue_denies_whole_set(pool: PgPool) {
    let f = Box::pin(Fixture::provisioning(&pool, 2)).await;
    revoke_second_mount(&f).await;
    f.denied_unchanged(&f.run().await).await;
    assert!(f.handoff_files().is_empty());
}
async fn pending_denial(f: &Fixture) {
    let session_id = capability_domain::RuntimeSessionId::from_uuid(f.birth.start.run_id.as_uuid());
    assert!(
        f.repository.find(session_id).await.is_err(),
        "genuine existing pending find failure must remain an error, never absence"
    );
    let rows = f.rows().await;
    assert_eq!(rows["runtime"]["sessions"].as_array().unwrap().len(), 1);
    let handoff = f.handoff_files();
    assert_eq!(handoff.len(), 1);
    f.denied_unchanged(&f.run().await).await;
    assert_eq!(f.rows().await, rows);
    assert_eq!(f.handoff_files(), handoff);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_source_withdrawn_pending_find_error_denies_replay_and_ack_then_scoped_revoke(
    pool: PgPool,
) {
    let f = Box::pin(Fixture::provisioning(&pool, 2)).await;
    f.issue().await;
    withdraw_source(&f).await;
    pending_denial(&f).await;
    let before = f.rows().await;
    f.manager()
        .revoke_after_guest(f.birth.start.run_id)
        .await
        .unwrap();
    let revoked = f.session().await;
    assert_eq!(
        revoked.status,
        capability_domain::RuntimeSessionStatus::Revoked
    );
    assert!(f.handoff_files().is_empty());
    let after = f.rows().await;
    assert_eq!(
        after["prior"], before["prior"],
        "scoped revoke must retain every held fence"
    );
    for field in ["snapshots", "bindings", "git", "credentials"] {
        assert_eq!(after["runtime"][field], before["runtime"][field]);
    }
    let after = f.rows().await;
    f.manager()
        .revoke_after_guest(f.birth.start.run_id)
        .await
        .unwrap();
    assert_eq!(f.rows().await, after);
    f.birth.physical.assert_unchanged();
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_binding_issuer_withdrawn_pending_find_error_denies_replay_and_ack(pool: PgPool) {
    let f = Box::pin(Fixture::provisioning(&pool, 2)).await;
    f.issue().await;
    withdraw_issuer(&f).await;
    pending_denial(&f).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_second_mount_revoked_pending_find_error_denies_replay_and_ack(pool: PgPool) {
    let f = Box::pin(Fixture::provisioning(&pool, 2)).await;
    f.issue().await;
    revoke_second_mount(&f).await;
    pending_denial(&f).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_current_execute_withdrawal_denies_empty_provisioning_before_issue(pool: PgPool) {
    let f = Box::pin(Fixture::provisioning(&pool, 0)).await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.birth.birth.fixture.consuming_project)
        .bind(f.birth.birth.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let rights: (bool, bool) = sqlx::query_as("SELECT check_permission('user',$1::text,'can_execute','agent_instance',$2::text)=1,check_permission('user',$1::text,'can_use','release_agent',$3::text)=1")
        .bind(f.birth.birth.actor.user_id.as_uuid()).bind(f.birth.start.instance_id.as_uuid())
        .bind(f.birth.start.release_agent_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(rights, (false, true));
    f.denied_unchanged(&f.run().await).await;
    assert!(f.handoff_files().is_empty());
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn launcher_saved_source_withdrawal_denies_actual_empty_leasing_eligibility(pool: PgPool) {
    let f = Box::pin(Fixture::leasing(&pool, 0)).await;
    withdraw_source(&f).await;
    f.denied_unchanged(&f.run().await).await;
    assert!(f.handoff_files().is_empty());
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn launcher_second_mount_revocation_denies_actual_two_selection_leasing_eligibility(
    pool: PgPool,
) {
    let f = Box::pin(Fixture::leasing(&pool, 2)).await;
    revoke_second_mount(&f).await;
    f.denied_unchanged(&f.run().await).await;
    assert!(f.handoff_files().is_empty());
}
