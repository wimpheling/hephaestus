use super::{
    fixture::{BirthMode, Fixture},
    host,
};
use crate::fixtures;
use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, begin_actor_transaction};
use recipe_application::{DeploymentError, DiagnosticCode, InstallProgress, RemovalProgress};
use sqlx::PgPool;
use volume_trait::{
    OwnedProvisioningExpectation, OwnedVolumeRootHistory, VolumeError, VolumeMetadataRepository,
    VolumeOwnedProvisioningRepository, VolumeRootHistoryRepository,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL plus native e2fsprogs"]
async fn new_manager_retains_actual_allocated_birth_after_creator_and_source_withdrawal(
    pool: PgPool,
) {
    let f = Fixture::new(&pool, BirthMode::AllocatedWithdrawal).await;
    let (identity, request) = f.remove().await;
    assert_source_use_denied(&f, &identity).await;
    let before = host::tree(f.root.path());
    let progress = f
        .coordinator()
        .retain(&identity, &request, &f.claim)
        .await
        .unwrap();
    assert_eq!(progress.install, InstallProgress::Creating);
    assert_eq!(progress.removal, RemovalProgress::Retained);
    assert_eq!(progress.active_attempt, None);
    assert_eq!(progress.version, request.expected_resource_version + 1);
    assert_eq!(host::tree(f.root.path()), before);
    assert_eq!(f.format_count(), 0);
    let fence: bool = sqlx::query_scalar("SELECT NOT owned_birth_is_live($1)")
        .bind(
            f.request
                .purpose
                .receipt()
                .intent
                .registration()
                .id()
                .as_uuid(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(fence);
    // This query proves immutable metadata visibility after actual atomic
    // retirement, not startup bootstrap authority or a new physical owner.
    assert_eq!(
        f.worker
            .owned_root_history(f.claim.purpose.host(), f.claim.purpose.root())
            .await
            .unwrap(),
        OwnedVolumeRootHistory::Existing {
            namespace: f.claim.purpose.owner_namespace(),
        }
    );
    assert_retired_discovery_readonly(&f).await;
    let mut fresh = identity.clone();
    fresh.request_id = identity_domain::RequestId::new();
    assert_eq!(
        f.coordinator()
            .retain(&fresh, &request, &f.claim)
            .await
            .unwrap(),
        progress
    );
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.seeded.consuming_project)
        .bind(f.seeded.outsider.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        f.coordinator().retain(&fresh, &request, &f.claim).await,
        Err(crate::coordinator::PartialRetentionError::Deployment(
            DeploymentError::AuthorizationDenied
        ))
    ));
    assert_eq!(host::tree(f.root.path()), before);
    assert!(
        f.worker
            .owned_reconciliation_context(&f.claim)
            .await
            .is_err()
    );
    assert!(
        f.worker
            .mark_ready(f.request.purpose.receipt().intent.registration().id())
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL plus native e2fsprogs"]
async fn actual_incomplete_format_is_retained_unusable_without_ready_or_absence_claim(
    pool: PgPool,
) {
    let f = Fixture::new(&pool, BirthMode::FormatIncomplete).await;
    let ambiguous = f
        .recipe
        .mark_resource_ambiguous(
            &f.identity,
            &f.target,
            DiagnosticCode::ProviderOutcomeUnknown,
        )
        .await
        .unwrap();
    let (identity, request) = f.remove().await;
    let before = host::tree(f.root.path());
    let progress = f
        .coordinator()
        .retain(&identity, &request, &f.claim)
        .await
        .unwrap();
    assert_eq!(progress.removal, RemovalProgress::Retained);
    assert_eq!(progress.install, ambiguous.install);
    assert_eq!(progress.install, InstallProgress::RecoveryRequired);
    assert_eq!(progress.diagnostic, ambiguous.diagnostic);
    assert_eq!(
        progress.diagnostic,
        Some(DiagnosticCode::ProviderOutcomeUnknown)
    );
    assert_eq!(f.format_count(), 1);
    assert_eq!(host::tree(f.root.path()), before);
    let phase: String =
        sqlx::query_scalar("SELECT phase FROM volume_owned_partial_receipts WHERE volume_id=$1")
            .bind(
                f.request
                    .purpose
                    .receipt()
                    .intent
                    .registration()
                    .id()
                    .as_uuid(),
            )
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(phase, "format_intent_incomplete");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL plus native e2fsprogs"]
async fn clean_real_ready_commit_gap_refuses_partial_and_reconciles_readonly(pool: PgPool) {
    let f = Fixture::new(&pool, BirthMode::ReadyGap).await;
    let (identity, request) = f.remove().await;
    let before = host::tree(f.root.path());
    assert!(
        f.coordinator()
            .retain(&identity, &request, &f.claim)
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_birth_retirements")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let ready = f.store.reconcile_owned(&f.claim).await.unwrap();
    assert_eq!(
        ready.observation.unwrap().fields().phase,
        volume_trait::OwnedBackingPhase::Ready
    );
    assert_eq!(host::tree(f.root.path()), before);
    assert_eq!(f.format_count(), 1);
    let snapshot = f.recipe.inspect(&identity, f.intent.id()).await.unwrap();
    assert_eq!(
        snapshot.resources[&fixtures::key("data")].active_attempt,
        Some(f.target.provenance.attempt_id)
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL plus native e2fsprogs"]
async fn unrelated_unremoved_bound_consumer_is_rejected_without_wildcard_cleanup(pool: PgPool) {
    let f = Fixture::new(&pool, BirthMode::FormatIncomplete).await;
    let consumer = bind_unremoved_consumer(&f).await;
    let (manager, request) = f.remove().await;
    let before = host::tree(f.root.path());
    let head: i64 =
        sqlx::query_scalar("SELECT version FROM volume_owned_observation_heads WHERE volume_id=$1")
            .bind(
                f.claim
                    .purpose
                    .receipt()
                    .intent
                    .registration()
                    .id()
                    .as_uuid(),
            )
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        f.coordinator()
            .retain(&manager, &request, &f.claim)
            .await
            .is_err()
    );
    let gate: bool = sqlx::query_scalar("SELECT run_gate_open FROM agent_instances WHERE id=$1")
        .bind(consumer)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!gate, "unsupported dispatch is not permanent removal proof");
    let closures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=$1",
    )
    .bind(consumer)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(closures, 0);
    assert_eq!(f.format_count(), 1);
    assert_eq!(host::tree(f.root.path()), before);
    let after_head: i64 =
        sqlx::query_scalar("SELECT version FROM volume_owned_observation_heads WHERE volume_id=$1")
            .bind(
                f.claim
                    .purpose
                    .receipt()
                    .intent
                    .registration()
                    .id()
                    .as_uuid(),
            )
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after_head, head);
    let fences: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_birth_retirements")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fences, 0);
    let snapshot = f.recipe.inspect(&manager, f.intent.id()).await.unwrap();
    assert_eq!(
        snapshot.resources[&fixtures::key("data")].active_attempt,
        Some(f.target.provenance.attempt_id)
    );
}

async fn bind_unremoved_consumer(f: &Fixture) -> uuid::Uuid {
    let instance = uuid::Uuid::new_v4();
    let revision = uuid::Uuid::new_v4();
    let volume = f
        .claim
        .purpose
        .receipt()
        .intent
        .registration()
        .id()
        .as_uuid();
    sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state,volume_mode,run_gate_open) SELECT $1,project_id,family_id,'partial-related-consumer','active','named',false FROM agent_instances WHERE id=$2")
        .bind(instance).bind(f.seeded.instance).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,$2,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,true FROM agent_instance_revisions WHERE id=(SELECT active_revision_id FROM agent_instances WHERE id=$3)")
        .bind(revision).bind(instance).bind(f.seeded.instance).execute(&f.pool).await.unwrap();
    // Frozen requires_state without an authored slot uses the checked107
    // legacy declaration in named mode. This records selection only: no mount
    // grant, attachment, Ready promotion or runnable provider result is asserted.
    sqlx::query("INSERT INTO agent_instance_revision_volume_bindings(instance_revision_id,instance_id,project_id,release_agent_id,slot_key,volume_id,access_mode,guest_path,slot_required,minimum_capacity_bytes,provenance,created_by) VALUES($1,$2,$3,$4,'state',$5,'read_write','/var/lib/hephaestus',true,1,'legacy_declaration',$6)")
        .bind(revision).bind(instance).bind(f.seeded.consuming_project).bind(f.seeded.release_agent).bind(volume).bind(f.identity.user_id.as_uuid()).execute(&f.pool).await.expect("real catalog/revision/owner binding guards stay enabled");
    sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
        .bind(instance)
        .bind(revision)
        .execute(&f.pool)
        .await
        .unwrap();
    let (selected, gate): (uuid::Uuid, bool) = sqlx::query_as(
        "SELECT binding.volume_id,instance.run_gate_open FROM agent_instance_revision_volume_bindings binding JOIN agent_instances instance ON instance.id=binding.instance_id WHERE binding.instance_id=$1 AND binding.instance_revision_id=$2 AND binding.slot_key='state'",
    )
    .bind(instance).bind(revision).fetch_one(&f.pool).await.unwrap();
    assert_eq!(
        selected, volume,
        "consumer selects the exact physical partial birth"
    );
    assert!(!gate, "fixture preserves unsupported named dispatch");
    instance
}

async fn assert_source_use_denied(
    fixture: &Fixture,
    manager: &identity_domain::AuthenticatedIdentity,
) {
    let app = fixtures::app_pool(&fixture.pool).await;
    for identity in [&fixture.identity, manager] {
        let mut tx = begin_actor_transaction(&app, identity).await.unwrap();
        let decision = PostgresMelangeAuthorizer
            .check(
                &mut tx,
                Subject::User(identity.user_id),
                Permission::CanUse,
                ObjectRef::new(ObjectType::ReleaseAgent, fixture.seeded.release_agent),
            )
            .await
            .unwrap();
        assert!(
            !decision.is_allowed(),
            "original creator and cleanup manager cannot use the exact pinned release agent"
        );
        tx.rollback().await.unwrap();
    }
    app.close().await;
}

// A real committed Retain closes birth permanently; discovery must not offer a
// different operation or mistake the unchanged host namespace for live custody.
async fn assert_retired_discovery_readonly(f: &Fixture) {
    let expected = OwnedProvisioningExpectation::new(
        f.request.purpose.receipt().intent.clone(),
        f.claim.purpose.host().to_owned(),
        f.claim.purpose.root().to_path_buf(),
        f.claim.purpose.owner_namespace(),
    )
    .unwrap();
    let before = retained_metadata(f).await;
    let host_before = host::tree(f.root.path());
    assert!(matches!(
        f.worker.discover_owned_provisioning(&expected).await,
        Err(VolumeError::InvalidState(
            "owned birth is permanently retired"
        ))
    ));
    assert_eq!(retained_metadata(f).await, before);
    assert_eq!(host::tree(f.root.path()), host_before);
    assert_eq!(f.format_count(), 0);
}
async fn retained_metadata(f: &Fixture) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('volume',to_jsonb(volume),'retirement',(SELECT jsonb_agg(to_jsonb(retirement)) FROM volume_owned_birth_retirements retirement WHERE retirement.volume_id=volume.id),'head',(SELECT jsonb_agg(to_jsonb(head)) FROM volume_owned_observation_heads head WHERE head.volume_id=volume.id),'observations',(SELECT jsonb_agg(to_jsonb(observation) ORDER BY observation.id) FROM volume_owned_backing_observations observation WHERE observation.volume_id=volume.id),'event_count',(SELECT count(*) FROM application_events)) FROM agent_instance_state_volumes volume WHERE volume.id=$1")
        .bind(f.request.purpose.receipt().intent.registration().id().as_uuid())
        .fetch_one(&f.pool).await.unwrap()
}
