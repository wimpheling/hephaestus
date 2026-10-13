use super::fixture::Fixture;
use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use run_domain::{RunOutcome, RunState};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, LeaseId,
    ReleaseAgentId, ReleaseId, RunId, VolumeId,
};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn exact_run_pins_and_forbidden_supplied_history_are_checked(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let mut pins = Vec::new();
    macro_rules! changed {
        ($field:ident,$value:expr) => {{
            let mut r = f.run.clone();
            r.$field = $value;
            pins.push(r);
        }};
    }
    changed!(id, RunId::new());
    changed!(instance_id, AgentInstanceId::from_uuid(Uuid::new_v4()));
    changed!(
        instance_revision_id,
        AgentInstanceRevisionId::from_uuid(Uuid::new_v4())
    );
    changed!(release_id, ReleaseId::from_uuid(Uuid::new_v4()));
    changed!(release_agent_id, ReleaseAgentId::from_uuid(Uuid::new_v4()));
    changed!(command_id, CommandId::new());
    changed!(requires_state, !f.run.requires_state);
    changed!(vm_id, Some("foreign-vm".to_owned()));
    changed!(state_version, f.run.state_version + 1);
    assert_eq!(pins.len(), 9);
    for r in pins {
        f.denied(&r).await;
        assert!(
            !f.raw(
                &f.birth.birth.worker,
                &r,
                &f.config(),
                AUTHORIZATION_MODEL_VERSION
            )
            .await
            .unwrap()
        );
    }
    let mut supplied = Vec::new();
    macro_rules! forbidden {
        ($field:ident,$value:expr) => {{
            let mut r = f.run.clone();
            r.$field = $value;
            supplied.push(r);
        }};
    }
    forbidden!(vm_id, None);
    forbidden!(
        attachment_id,
        Some(AgentAttachmentId::from_uuid(Uuid::new_v4()))
    );
    forbidden!(volume_id, Some(VolumeId::from_uuid(Uuid::new_v4())));
    forbidden!(lease_id, Some(LeaseId::from_uuid(Uuid::new_v4())));
    forbidden!(lease_fencing_token, Some(1));
    forbidden!(cancel_requested_at, Some(super::fixture::pg_time()));
    forbidden!(outcome, Some(RunOutcome::Failed));
    forbidden!(
        exit,
        Some(vm_trait::VmExit {
            code: Some(0),
            signal: None
        })
    );
    forbidden!(failure, Some("supplied failure".to_owned()));
    for state in [
        RunState::Queued,
        RunState::Provisioning,
        RunState::Starting,
        RunState::Running,
        RunState::Succeeded,
        RunState::Failed,
        RunState::Cancelled,
        RunState::CleaningUp,
        RunState::CleanedUp,
    ] {
        forbidden!(state, state);
    }
    assert_eq!(supplied.len(), 18);
    for r in supplied {
        f.denied(&r).await;
    }
    f.allowed().await;
}
