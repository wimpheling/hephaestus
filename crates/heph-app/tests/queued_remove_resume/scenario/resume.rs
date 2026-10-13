//! Existing same-original/fresh-manager resume and pending completion phases.
use super::{History, denial, fresh};
use crate::{assertions, fixture::Fixture, instance};
use hephaestus_app::ManagedInvocationController;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentOperation, EffectClaim, ExistingInstanceClosureRepository,
    ExistingInstanceClosureRequest, PlatformPolicyObservation, PreparedInstanceClosureAdmission,
    RemoveDeployment, RemoveResumeConfiguration,
};
use recipe_postgres::PostgresDeploymentRepository;
use release_postgres::PostgresPreparedInstanceSafetyRepository;
use run_domain::{CancelRun, StartRun};
use serde_json::Value;
use std::sync::Arc;

pub(super) async fn resume_original(
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

pub(super) async fn finish_pending(
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
