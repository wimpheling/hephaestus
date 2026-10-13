use super::denial;
use crate::{assertions, fixture::Fixture, instance, queued, volume};
use hephaestus_app::ManagedInvocationController;
use identity_domain::{AuthenticatedIdentity, RequestId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentOperation, EffectClaim,
    ExistingInstanceClosureRepository, ExistingInstanceClosureRequest, PlatformPolicyObservation,
    PreparedInstanceClosureAdmission, PreparedInstanceClosureRepository,
    PreparedInstanceClosureRequest, PreparedInstanceSafetyRepository, RemoveDeployment,
    RemoveResumeConfiguration, ResourceAction, ResourceEffect,
};
use recipe_postgres::PostgresDeploymentRepository;
use release_postgres::PostgresPreparedInstanceSafetyRepository;
use run_domain::{CancelRun, StartRun};
use runtime_types::VolumeId;
use serde_json::Value;
use std::sync::Arc;

fn fresh(actor: &AuthenticatedIdentity) -> AuthenticatedIdentity {
    let mut fresh = actor.clone();
    fresh.request_id = RequestId::new();
    fresh.idempotency_id = RequestId::new();
    fresh
}

struct History {
    starts: [StartRun; 2],
    ordinary: CancelRun,
    completed: Value,
}

pub async fn run(fixture: &Fixture, empty: bool) {
    let prepared = instance::prepare(fixture).await;
    let volume = volume::create(fixture, &prepared).await;
    instance::import(fixture, &prepared).await;
    let controller = fixture.controller().await;
    let history = if empty {
        None
    } else {
        Some(ordinary_before_remove(fixture, &prepared, volume, &controller).await)
    };
    let (repository, safety) = repositories(fixture);
    let (closed, drain) = close_original(fixture, &prepared, &repository, &safety).await;
    let physical = denial::Backing::capture(fixture, volume).await;
    let original = resume_original(
        fixture,
        &prepared,
        &repository,
        &safety,
        &closed,
        &drain,
        history.as_ref().map(|h| &h.starts),
    )
    .await;
    if let Some(history) = &history {
        finish_pending(
            fixture,
            &prepared,
            &repository,
            &controller,
            history,
            &original,
        )
        .await;
    }
    physical.assert_preserved(fixture).await;
    assert!(
        fixture
            .provider
            .attempt_inventory(fixture.config.provider_scope())
            .await
            .unwrap()
            .attempts
            .is_empty()
    );
    drop(controller);
}

async fn ordinary_before_remove(
    fixture: &Fixture,
    prepared: &instance::Prepared,
    volume: VolumeId,
    controller: &Arc<ManagedInvocationController>,
) -> History {
    let starts = queued::activate_and_invoke(fixture, prepared).await;
    let before = assertions::Snapshot::capture(fixture, &prepared.instance, &starts, volume).await;
    before.require_nonempty();
    let ordinary = queued::cancel(
        fixture,
        &prepared.actor,
        starts[0].run_id,
        "ordinary before genuine removal",
    )
    .await;
    assert!(controller.clone().cancel_run(&ordinary).await.unwrap());
    assertions::cleaned(fixture, controller, starts[0].run_id).await;
    before
        .assert_preserved(fixture, &prepared.instance, &starts)
        .await;
    let completed = assertions::completion(fixture, starts[0].run_id).await;
    History {
        starts,
        ordinary,
        completed,
    }
}

fn repositories(
    fixture: &Fixture,
) -> (
    PostgresDeploymentRepository,
    PostgresPreparedInstanceSafetyRepository,
) {
    let platform = PlatformPolicyObservation::new(
        fixture.config.platform_policy().clone(),
        fixture.config.platform_version().into(),
    )
    .unwrap();
    let config = RemoveResumeConfiguration::new(
        fixture.config.provider_scope().namespace().into(),
        fixture.config.provider_scope().host_id().into(),
        platform.clone(),
        recipe_application::DeploymentExecutionProfile::RuntimeNamedV1,
    )
    .unwrap();
    let repository = PostgresDeploymentRepository::new(fixture.original.application.clone())
        .with_remove_resume(config.clone());
    let safety = PostgresPreparedInstanceSafetyRepository::new(
        fixture.original.application.clone(),
        fixture.original.worker.clone(),
        fixture.config.provider_scope(),
        platform,
    )
    .with_remove_resume(config)
    .unwrap();
    (repository, safety)
}

async fn close_original(
    fixture: &Fixture,
    prepared: &instance::Prepared,
    repository: &PostgresDeploymentRepository,
    safety: &PostgresPreparedInstanceSafetyRepository,
) -> (PreparedInstanceClosureAdmission, EffectClaim) {
    let remover = fresh(&prepared.actor);
    let remove_command =
        CommandIdentity::from_identity(&remover, DeploymentOperation::Remove).unwrap();
    let snapshot = repository
        .inspect(&remover, prepared.deployment)
        .await
        .unwrap();
    let admission = repository
        .admit_remove(
            &remover,
            RemoveDeployment {
                command: remove_command,
                deployment_id: prepared.deployment,
                expected_version: snapshot.version,
            },
        )
        .await
        .unwrap();
    let resource = instance::key("agent");
    let target = safety
        .load_original_target(
            &remover,
            remove_command,
            &admission.snapshot.intent,
            &resource,
        )
        .await
        .unwrap();
    let drain = repository
        .claim_resource_effect(
            &remover,
            ResourceEffect {
                command: remove_command,
                deployment_id: prepared.deployment,
                resource: resource.clone(),
                expected_deployment_version: admission.snapshot.version,
                expected_resource_version: admission.snapshot.resources[&resource].version,
                action: ResourceAction::Drain,
                provenance: AttemptProvenance::new(
                    &remover,
                    DeploymentAttemptId::from_uuid(uuid::Uuid::new_v4()).unwrap(),
                )
                .unwrap(),
            },
        )
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("SELECT version FROM agent_instances WHERE id=$1")
        .bind(prepared.instance.instance_id().as_uuid())
        .fetch_one(&fixture.admin)
        .await
        .unwrap();
    let closure = PreparedInstanceClosureRequest::new(
        admission.snapshot.intent.clone(),
        target.clone(),
        drain.clone(),
        u64::try_from(version).unwrap(),
    )
    .unwrap();
    let closed = safety.close_for_remove(&remover, &closure).await.unwrap();
    assert!(!closed.replayed);
    let revoked: (bool, i64, i64) = sqlx::query_as("SELECT NOT run_gate_open,(SELECT count(*) FROM agent_instance_volume_mount_grants WHERE instance_id=$1),(SELECT count(*) FROM agent_instance_volume_mount_revocations revocation JOIN agent_instance_volume_mount_grants grant_row ON grant_row.id=revocation.grant_id WHERE grant_row.instance_id=$1) FROM agent_instances WHERE id=$1")
        .bind(prepared.instance.instance_id().as_uuid()).fetch_one(&fixture.admin).await.unwrap();
    assert!(revoked.0);
    assert_eq!(revoked.1, 1);
    assert_eq!(revoked.2, revoked.1);
    (closed, drain)
}

async fn resume_original(
    fixture: &Fixture,
    prepared: &instance::Prepared,
    repository: &PostgresDeploymentRepository,
    safety: &PostgresPreparedInstanceSafetyRepository,
    closed: &PreparedInstanceClosureAdmission,
    drain: &EffectClaim,
    starts: Option<&[StartRun; 2]>,
) -> recipe_application::OriginalInstanceClosure {
    let before = denial::inventory(&fixture.admin).await;
    let manager = fresh(&prepared.actor);
    let manager_command =
        CommandIdentity::from_identity(&manager, DeploymentOperation::Remove).unwrap();
    let current = repository
        .inspect(&manager, prepared.deployment)
        .await
        .unwrap();
    let remove = RemoveDeployment {
        command: manager_command,
        deployment_id: prepared.deployment,
        expected_version: current.version,
    };
    let admission = repository
        .admit_remove_resume(&manager, remove)
        .await
        .unwrap();
    assert_eq!(admission.continuations().len(), 1);
    let original = admission.continuations()[0]
        .closure
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(original.drain, *drain);
    assert_eq!(original.removal_id, closed.removal_id);
    let request = ExistingInstanceClosureRequest::new(
        manager_command,
        current.intent.clone(),
        closed.target.clone(),
        original.clone(),
    )
    .unwrap();
    assert_eq!(
        safety
            .load_existing_instance_closure(&manager, &request)
            .await
            .unwrap(),
        original
    );
    let replay = repository
        .admit_remove_resume(&manager, remove)
        .await
        .unwrap();
    assert_eq!(replay.continuations(), admission.continuations());
    assert_eq!(replay.admission().receipt, admission.admission().receipt);
    assert_eq!(before, denial::inventory(&fixture.admin).await);
    deny_foreign_config(fixture, &manager, remove).await;
    denial::cases(fixture, &manager, &request, starts).await;
    original
}

async fn finish_pending(
    fixture: &Fixture,
    prepared: &instance::Prepared,
    repository: &PostgresDeploymentRepository,
    controller: &Arc<ManagedInvocationController>,
    history: &History,
    original: &recipe_application::OriginalInstanceClosure,
) {
    let starts = &history.starts;
    assert_eq!(
        &history.completed,
        &assertions::completion(fixture, starts[0].run_id).await
    );
    // Read ONLY the DTO emitted by the actual protected closure producer.
    let dto: Value = sqlx::query_scalar("SELECT payload-ARRAY['schema_version','message_id','idempotency_key','request_id','trace_id'] FROM outbox WHERE aggregate_type='release' AND aggregate_id=$1 AND event_type='run.cancel_requested.v1'")
        .bind(starts[1].run_id.as_uuid()).fetch_one(&fixture.admin).await.unwrap();
    let cancel: CancelRun = serde_json::from_value(dto).unwrap();
    assert_eq!(cancel.run_id, starts[1].run_id);
    assert!(controller.clone().cancel_run(&cancel).await.unwrap());
    assertions::cleaned(fixture, controller, starts[1].run_id).await;
    assert!(
        !controller
            .clone()
            .cancel_run(&history.ordinary)
            .await
            .unwrap()
    );
    assert_eq!(
        &history.completed,
        &assertions::completion(fixture, starts[0].run_id).await
    );
    let newer = fresh(&prepared.actor);
    let newer_command =
        CommandIdentity::from_identity(&newer, DeploymentOperation::Remove).unwrap();
    let snapshot = repository
        .inspect(&newer, prepared.deployment)
        .await
        .unwrap();
    let resumed = repository
        .admit_remove_resume(
            &newer,
            RemoveDeployment {
                command: newer_command,
                deployment_id: prepared.deployment,
                expected_version: snapshot.version,
            },
        )
        .await
        .unwrap();
    assert_eq!(resumed.continuations()[0].closure.as_ref(), Some(original));
    for start in &history.starts {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM run_invocation_consumptions WHERE run_id=$1")
                .bind(start.run_id.as_uuid())
                .fetch_one(&fixture.admin)
                .await
                .unwrap();
        assert_eq!(count, 0);
    }
}

async fn deny_foreign_config(
    fixture: &Fixture,
    manager: &AuthenticatedIdentity,
    remove: RemoveDeployment,
) {
    let before = denial::inventory(&fixture.admin).await;
    let platform = PlatformPolicyObservation::new(
        fixture.config.platform_policy().clone(),
        fixture.config.platform_version().into(),
    )
    .unwrap();
    let wrong = RemoveResumeConfiguration::new(
        "foreign-owner".into(),
        fixture.config.provider_scope().host_id().into(),
        platform,
        recipe_application::DeploymentExecutionProfile::RuntimeNamedV1,
    )
    .unwrap();
    let repository = PostgresDeploymentRepository::new(fixture.original.application.clone())
        .with_remove_resume(wrong);
    assert!(
        repository
            .admit_remove_resume(manager, remove)
            .await
            .is_err()
    );
    assert_eq!(before, denial::inventory(&fixture.admin).await);
}
