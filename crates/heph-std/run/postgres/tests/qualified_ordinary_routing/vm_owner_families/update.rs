//! Parent-only released candidate plus actual current-authorized update commands.

use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::{AgentUpdateId, NetworkAccess, ReleaseCommandKey, RuntimePolicy};
use release_postgres::ReleaseService;
use release_service::{BeginUpdateHook, CreateInstanceUpdate};
use run_domain::{
    CancelRun, FreshRunExecutionProfile, LegacyVmPlacementScope, RunCleanupHostId, RunState,
    StartRun,
};
use run_orchestrator::RunRepository;
use run_postgres::PgRunRepository;
use runtime_types::{AgentInstanceRevisionId, CommandId, RunId};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;
use vm_trait::VmId;

pub(super) struct Fixture {
    pub(super) original: super::super::fixture::Fixture,
    pub(super) service: ReleaseService,
    pub(super) runs: PgRunRepository,
    pub(super) start: StartRun,
    pub(super) update: AgentUpdateId,
    profile: FreshRunExecutionProfile,
}

impl Fixture {
    pub(super) async fn new(pool: &PgPool, profile: FreshRunExecutionProfile) -> Self {
        let original = super::super::fixture::Fixture::new(pool).await;
        // Drain the actual produced Normal before requesting the new hook;
        // neither deleting its request nor inventing a cleaned Run is used.
        original.birth().await;
        cancel(&original.runs, &original.start).await;
        finish_pre_io(&original.runs, &original.start, &original.scope).await;
        let actor = actor(pool, &original).await;
        let candidate_agent = candidate(pool, &original).await;
        let update = request_update(&original, &actor, candidate_agent).await;
        let mut service =
            ReleaseService::new(original.worker.clone(), Arc::new(PostgresMelangeAuthorizer))
                .with_execution_profile(profile);
        let mut runs =
            PgRunRepository::new(original.worker.clone()).with_execution_profile(profile);
        match profile {
            FreshRunExecutionProfile::LegacyScalar => {
                service = service.with_legacy_vm_placement(original.scope.clone());
                runs = runs.with_legacy_vm_placement(original.scope.clone());
            }
            FreshRunExecutionProfile::OwnedCanonical => {
                runs = runs.with_vm_owner_scope(cleanup_scope(&original.scope));
            }
        }
        let id = RunId::new();
        service
            .begin_update_hook(
                &actor,
                BeginUpdateHook {
                    command_key: ReleaseCommandKey::derive(
                        "owner-history-hook",
                        &[id.as_uuid().as_bytes()],
                    ),
                    update_id: update,
                    hook_run_id: id,
                },
            )
            .await
            .unwrap();
        let run = runs.get(id).await.unwrap();
        let start = StartRun {
            command_id: run.command_id,
            run_id: run.id,
            instance_id: run.instance_id,
            instance_revision_id: run.instance_revision_id,
            release_id: run.release_id,
            release_agent_id: run.release_agent_id,
            attachment_id: run.attachment_id,
            kind: run.kind,
            requires_state: run.requires_state,
        };
        Self {
            original,
            service,
            runs,
            start,
            update,
            profile,
        }
    }

    pub(super) async fn consume(&self) {
        let created = match self.profile {
            FreshRunExecutionProfile::LegacyScalar => self
                .runs
                .create_run_with_legacy_placement(&self.start, &self.original.scope)
                .await
                .unwrap(),
            FreshRunExecutionProfile::OwnedCanonical => self
                .runs
                .create_run_with_vm_plan(
                    &self.start,
                    &cleanup_scope(&self.original.scope),
                    &VmId(self.start.run_id.to_string()),
                )
                .await
                .unwrap(),
        };
        assert!(created.created);
    }

    pub(super) async fn cancel(&self) {
        cancel(&self.runs, &self.start).await;
    }

    pub(super) async fn assert_uncertain(&self, pool: &PgPool) {
        let state: String = sqlx::query_scalar("SELECT state FROM agent_updates WHERE id=$1")
            .bind(self.update.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(state, "compatibility_unknown");
    }

    pub(super) async fn assert_original_command_cannot_disappear(&self) {
        let error = sqlx::query("DELETE FROM release_command_inbox WHERE operation='begin_update_hook' AND secondary_id=$1")
            .bind(self.start.run_id.as_uuid()).execute(&self.original.worker).await.unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("23000")
        );
    }
}

fn cleanup_scope(scope: &LegacyVmPlacementScope) -> RunCleanupHostId {
    RunCleanupHostId::new(
        scope.provider_scope().namespace().into(),
        scope.provider_scope().host_id().into(),
    )
    .unwrap()
}

async fn cancel(runs: &PgRunRepository, start: &StartRun) {
    assert!(
        runs.request_cancel(&CancelRun {
            command_id: CommandId::new(),
            run_id: start.run_id,
            reason: "original-owner observation before provider IO".into(),
        })
        .await
        .unwrap()
    );
    runs.transition(start.run_id, RunState::Cancelled, None, None)
        .await
        .unwrap();
}

pub(super) async fn finish_pre_io(
    runs: &PgRunRepository,
    start: &StartRun,
    scope: &LegacyVmPlacementScope,
) {
    runs.close_legacy_vm_acquisition(start.run_id, scope)
        .await
        .unwrap();
    for state in [RunState::CleaningUp, RunState::CleanedUp] {
        runs.transition(start.run_id, state, None, None)
            .await
            .unwrap();
    }
}

async fn actor(pool: &PgPool, f: &super::super::fixture::Fixture) -> AuthenticatedIdentity {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'Owner history producer manager')")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) SELECT project_id,$2 FROM agent_instances WHERE id=$1")
        .bind(f.start.instance_id.as_uuid()).bind(id).execute(pool).await.unwrap();
    AuthenticatedIdentity::new(
        UserId::from_uuid(id),
        "owner-history-fixture",
        id.to_string(),
        serde_json::json!({}),
        RequestId::new(),
    )
}

async fn candidate(pool: &PgPool, f: &super::super::fixture::Fixture) -> Uuid {
    // Only released parent metadata is seeded. All revision/update/command/Run/
    // admission/placement rows below it are produced by the actual services.
    let release = Uuid::new_v4();
    let agent = Uuid::new_v4();
    let contract = serde_json::json!({"policy_ceiling":{"vcpus":1,"memory_mib":512,"network":"disabled"},"volume_slots":[]});
    let hash: [u8; 32] = Sha256::digest(contract.to_string().as_bytes()).into();
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state,published_at) SELECT $1,repository_id,'owner-history-update-v2',source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'published',now() FROM releases WHERE id=$2")
        .bind(release).bind(f.start.release_id.as_uuid()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) SELECT $1,$2,family_id,agent_key,'Owner history Update',$3,$4,false FROM release_agents WHERE id=$5")
        .bind(agent).bind(release).bind(contract).bind(hash.as_slice()).bind(f.start.release_agent_id.as_uuid()).execute(pool).await.unwrap();
    agent
}

async fn request_update(
    original: &super::super::fixture::Fixture,
    actor: &AuthenticatedIdentity,
    candidate_agent: Uuid,
) -> AgentUpdateId {
    let update = AgentUpdateId::new();
    let policy = RuntimePolicy {
        vcpus: 1,
        memory_mib: 512,
        network: NetworkAccess::Disabled,
    };
    ReleaseService::new(original.worker.clone(), Arc::new(PostgresMelangeAuthorizer))
        .create_update(
            actor,
            CreateInstanceUpdate {
                command_key: ReleaseCommandKey::derive(
                    "owner-history-update",
                    &[update.as_uuid().as_bytes()],
                ),
                update_id: update,
                instance_id: original.start.instance_id,
                expected_revision_id: original.start.instance_revision_id,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: candidate_agent.into(),
                parameters: BTreeMap::new(),
                brokered_rule_copies: Vec::new(),
                selected_policy: policy.clone(),
                platform_policy: policy,
                platform_policy_version: "test/v1".into(),
            },
        )
        .await
        .unwrap();
    update
}
