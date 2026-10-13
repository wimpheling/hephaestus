use super::{execution::Execution, preparation};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, begin_actor_transaction};
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, OwnedReadyRetentionRepository,
    ReopenRetainedOwnedVolume, ResourceAction, ResourceEffect,
};
use run_domain::RunState;
use run_orchestrator::RunRepository;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine116/119 PostgreSQL and actual native ext4"]
async fn retained_completed_birth_reopens_for_current_manager_and_exact119_replay_preserves_history(
    pool: PgPool,
) {
    let f = crate::retained_fixture::Fixture::new(&pool, true).await;
    let original = f.original().await;
    let old_creator = f.birth.h.actor.clone();
    let user = f.birth.h.fixture.outsider;
    f.manager(user).await;
    f.withdraw_installer().await;
    let mut tx = begin_actor_transaction(&f.birth.h.app, &old_creator)
        .await
        .unwrap();
    assert_eq!(
        PostgresMelangeAuthorizer
            .check(
                &mut tx,
                Subject::User(old_creator.user_id),
                Permission::CanUse,
                ObjectRef::new(ObjectType::ReleaseAgent, f.birth.h.fixture.release_agent)
            )
            .await
            .unwrap(),
        AuthorizationDecision::Deny
    );
    tx.rollback().await.unwrap();
    let (manager, request) = f.remove(user).await;
    assert_ne!(manager.user_id, old_creator.user_id);
    f.coordinator()
        .retain(&manager, &request, &f.ready.claim)
        .await
        .unwrap();
    let bytes = std::fs::read(f.ready.claim.purpose.canonical_path()).unwrap();
    let original_counts = (f.birth.count("formats"), f.birth.count("fscks"));
    sqlx::query(
        "INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(f.birth.h.fixture.project)
    .bind(manager.user_id.as_uuid())
    .execute(&pool)
    .await
    .unwrap();
    let reuse = preparation::external_claim(&f, manager.user_id).await;
    let reopening = ReopenRetainedOwnedVolume {
        claim: reuse.claim.clone(),
        retention_operation_id: request.operation_id,
    };
    f.actor
        .reopen_retained_owned_volume(&reuse.identity, &reopening)
        .await
        .unwrap();
    let after = f.snapshot().await;
    f.actor
        .reopen_retained_owned_volume(&reuse.identity, &reopening)
        .await
        .unwrap();
    assert_eq!(f.snapshot().await, after);
    let execution = Execution::new(&f, &reuse).await;
    let runs = execution.repository(&f);
    let consumed = runs
        .create_run_with_vm_plan(&execution.start, &execution.scope, &execution.vm)
        .await
        .unwrap();
    assert!(!consumed.created);
    runs.transition(consumed.run.id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let volumes = PostgresVolumeMetadataRepository::new(f.birth.h.worker.clone())
        .with_qualified_invocation_configuration(execution.configuration.clone());
    let selected = volumes.load_run_selections(consumed.run.id).await.unwrap();
    assert_eq!(selected.selections().len(), 1);
    assert_eq!(
        selected.selections()[0].scope().volume_id(),
        f.ready.claim.purpose.receipt().intent.registration().id()
    );
    let now = time::OffsetDateTime::now_utc();
    let first = volumes
        .acquire_run(
            &selected,
            execution.scope.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    let exact = f.snapshot().await;
    let replay = volumes
        .acquire_run(
            &selected,
            execution.scope.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(
        f.snapshot().await,
        exact,
        "same held lease and current global fence; no generation reset"
    );
    assert_eq!(
        f.original().await,
        original,
        "original purpose/claim/provision bytes retained"
    );
    assert_eq!(
        std::fs::read(f.ready.claim.purpose.canonical_path()).unwrap(),
        bytes
    );
    assert_eq!(
        (f.birth.count("formats"), f.birth.count("fscks")),
        original_counts,
        "metadata acquisition does not format or fsck"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine116/119 PostgreSQL and actual native ext4"]
async fn closed_without_explicit_reopening_cannot_complete_dependency_or_import_or_acquire(
    pool: PgPool,
) {
    let f = crate::retained_fixture::Fixture::new(&pool, true).await;
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    f.coordinator()
        .retain(&identity, &request, &f.ready.claim)
        .await
        .unwrap();
    let bytes = std::fs::read(f.ready.claim.purpose.canonical_path()).unwrap();
    let reuse = preparation::external_claim(&f, identity.user_id).await;
    // No positive external verification is fabricated. Its active dependency
    // blocks even Instance/Create before a protected115/117 Run can exist.
    let snapshot = f
        .birth
        .h
        .repository
        .inspect(&identity, reuse.intent.id())
        .await
        .unwrap();
    let key = crate::fixtures::key("sqlite");
    let before = f.snapshot().await;
    assert!(
        f.birth
            .h
            .repository
            .claim_resource_effect(
                &identity,
                ResourceEffect {
                    command: reuse.command,
                    deployment_id: reuse.intent.id(),
                    resource: key.clone(),
                    expected_deployment_version: snapshot.version,
                    expected_resource_version: snapshot.resources[&key].version,
                    action: ResourceAction::Create,
                    provenance: AttemptProvenance::new(
                        &identity,
                        DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap()
                    )
                    .unwrap(),
                }
            )
            .await
            .is_err()
    );
    let service = release_postgres::ReleaseService::new(
        f.birth.h.app.clone(),
        Arc::new(PostgresMelangeAuthorizer),
    );
    assert!(
        service
            .import_agent_with_volumes(
                &identity,
                hephaestus_app::recipe_instance_import::to_import_command(&reuse.input)
            )
            .await
            .is_err()
    );
    super::execution::upgrade(&pool, 114).await;
    super::execution::upgrade(&pool, 115).await;
    super::execution::upgrade(&pool, 119).await;
    let none:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE volume_id=$1) AND NOT EXISTS(SELECT 1 FROM agent_instance_execution_invocations WHERE instance_id=$2)")
        .bind(f.ready.claim.purpose.receipt().intent.registration().id().as_uuid()).bind(reuse.input.instance_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(none);
    assert_eq!(f.snapshot().await, before);
    assert_eq!(
        std::fs::read(f.ready.claim.purpose.canonical_path()).unwrap(),
        bytes
    );
}
