use super::{fixture::Fixture, instance::Prepared};
use authz_postgres::PostgresMelangeAuthorizer;
use control_plane_postgres::run::{ControlKind, ControlTarget, RequestControl, RunApplication};
use forge_storage::GitStorage;
use identity_domain::{AuthenticatedIdentity, RequestId};
use release_domain::ReleaseCommandKey;
use release_postgres::PostgresInstanceExecutionService;
use release_service::{
    ActivateInstance, InstanceActivationId, InstanceExecutionService, InstanceInvocationId,
    InvokeInstance,
};
use review_domain::ControlCommand;
use review_git::GitReviewPublisher;
use review_postgres::{GitRepositoryLocator, PostgresReviewRepository};
use review_service::{ControlOutcome, ReviewControlService};
use run_domain::{CancelRun, StartRun};
use runtime_types::{CommandId, RunId};
use std::sync::Arc;
use uuid::Uuid;

fn fresh(actor: &AuthenticatedIdentity) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        actor.user_id,
        "ordinary-cancel",
        actor.user_id.to_string(),
        serde_json::json!({}),
        RequestId::new(),
    )
}

pub async fn activate_and_invoke(fixture: &Fixture, prepared: &Prepared) -> [StartRun; 2] {
    let service = PostgresInstanceExecutionService::new(
        fixture.original.application.clone(),
        Arc::new(PostgresMelangeAuthorizer),
        fixture.config.clone(),
    )
    .with_qualified_invocation_admission();
    let activation = Uuid::new_v4();
    let activated = service
        .activate_instance(
            &fresh(&prepared.actor),
            ActivateInstance::new(
                ReleaseCommandKey::derive("ordinary-cancel-activate", &[activation.as_bytes()]),
                InstanceActivationId::from_uuid(activation).unwrap(),
                prepared.instance.instance_id(),
                prepared.instance.revision_id(),
                1,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(activated.activated_version(), 2);
    let first = invoke(fixture, &service, prepared).await;
    let second = invoke(fixture, &service, prepared).await;
    assert_ne!(first.run_id, second.run_id);
    assert_ne!(first.command_id, second.command_id);
    [first, second]
}

async fn invoke(
    fixture: &Fixture,
    service: &PostgresInstanceExecutionService,
    prepared: &Prepared,
) -> StartRun {
    let id = Uuid::new_v4();
    let command = InvokeInstance::new(
        ReleaseCommandKey::derive("ordinary-cancel-invoke", &[id.as_bytes()]),
        InstanceInvocationId::from_uuid(id).unwrap(),
        RunId::new(),
        CommandId::new(),
        prepared.instance.instance_id(),
        prepared.instance.revision_id(),
    )
    .unwrap();
    service
        .invoke_instance(&fresh(&prepared.actor), command)
        .await
        .unwrap();
    let value: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM command_inbox WHERE command_id=$1")
            .bind(command.start_command_id().as_uuid())
            .fetch_one(&fixture.admin)
            .await
            .unwrap();
    let start: StartRun = serde_json::from_value(value).unwrap();
    assert_eq!(start.run_id, command.run_id());
    assert_eq!(start.instance_id, prepared.instance.instance_id());
    assert_eq!(start.instance_revision_id, prepared.instance.revision_id());
    assert!(start.requires_state);
    assert!(start.attachment_id.is_none());
    start
}

pub async fn cancel(
    fixture: &Fixture,
    actor: &AuthenticatedIdentity,
    run: RunId,
    reason: &str,
) -> CancelRun {
    let request = RunApplication::new(
        fixture.original.application.clone(),
        fixture.run_runtime.runtime_root.clone(),
    )
    .request_control(
        &fresh(actor),
        RequestControl {
            kind: ControlKind::Cancel,
            repository_id: fixture.original.repository,
            target: ControlTarget::Run(run.as_uuid()),
            reason: reason.into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(request.state, "pending");
    let control:serde_json::Value=sqlx::query_scalar("SELECT payload FROM outbox WHERE aggregate_type='control_request' AND aggregate_id=$1 AND subject='hephaestus.control.execute'").bind(request.id).fetch_one(&fixture.admin).await.unwrap();
    let control: ControlCommand = serde_json::from_value(control).unwrap();
    assert_eq!(control.run_id, Some(run));
    assert_eq!(control.reason, reason);
    let storage = Arc::new(
        GitStorage::initialize(fixture.run_runtime.release_artifact_root.join("review-git"))
            .await
            .unwrap(),
    );
    let service = ReviewControlService::with_git(
        // Match the trusted review consumer in build_preparation: browser
        // issuance stays on the application pool; consumption uses the owner.
        Arc::new(PostgresReviewRepository::new(fixture.admin.clone())),
        Arc::new(GitReviewPublisher::new(Arc::new(
            GitRepositoryLocator::new(storage),
        ))),
    );
    assert_delivery_guard(fixture, &service, &control).await;
    assert_eq!(
        service.execute(&control).await.unwrap(),
        ControlOutcome::Completed
    );
    assert_eq!(
        service.execute(&control).await.unwrap(),
        ControlOutcome::AlreadyCompleted
    );
    let dto:serde_json::Value=sqlx::query_scalar("SELECT payload FROM outbox WHERE aggregate_type='run' AND aggregate_id=$1 AND subject='heph.run.command.cancel.v1' AND payload->>'command_id'=$2").bind(run.as_uuid()).bind(request.id.to_string()).fetch_one(&fixture.admin).await.unwrap();
    let cancel: CancelRun = serde_json::from_value(dto).unwrap();
    assert_eq!(cancel.command_id.as_uuid(), request.id);
    assert_eq!(cancel.run_id, run);
    assert_eq!(cancel.reason, reason);
    cancel
}

async fn assert_delivery_guard(
    fixture: &Fixture,
    service: &ReviewControlService,
    command: &ControlCommand,
) {
    let original = control_effects(fixture, command).await;
    let mut changed = command.clone();
    changed.request_id = RequestId::new();
    assert!(matches!(
        service.execute(&changed).await,
        Err(review_service::ControlServiceError::Repository(
            review_service::ReviewRepositoryError::DeliveryMismatch
        ))
    ));
    assert_eq!(original, control_effects(fixture, command).await);
}

async fn control_effects(fixture: &Fixture, command: &ControlCommand) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('control', to_jsonb(control), 'outbox',
            (SELECT coalesce(jsonb_agg(to_jsonb(event) ORDER BY event.id), '[]'::jsonb)
             FROM outbox event WHERE event.aggregate_id = control.id
                OR (event.aggregate_type = 'run'
                    AND event.payload->>'command_id' = control.id::text)))
         FROM control_requests control WHERE control.id = $1",
    )
    .bind(command.command_id.as_uuid())
    .fetch_one(&fixture.admin)
    .await
    .unwrap()
}
