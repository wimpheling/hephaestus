//! Actual activation and Invocation admission precede worker consumption.

use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::AuthenticatedIdentity;
use recipe_application::PreparedInstanceImport;
use release_domain::ReleaseCommandKey;
use release_postgres::PostgresInstanceExecutionService;
use release_service::{
    ActivateInstance, InstanceActivationId, InstanceExecutionConfiguration,
    InstanceExecutionService, InstanceInvocationId, InvokeInstance,
};
use run_domain::{FreshRunExecutionProfile, RunCleanupHostId, RunKind, StartRun};
use run_postgres::PgRunRepository;
use runtime_types::{CommandId, RunId};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::fixture::Fixture;

pub(super) async fn admit(
    pool: &PgPool,
    original: &Fixture,
    actor: &AuthenticatedIdentity,
    prepared: &PreparedInstanceImport,
    config: &InstanceExecutionConfiguration,
) -> (StartRun, PgRunRepository, RunCleanupHostId) {
    let service = PostgresInstanceExecutionService::new(
        original.application.clone(),
        Arc::new(PostgresMelangeAuthorizer),
        config.clone(),
    )
    .with_qualified_invocation_admission();
    let activation = Uuid::new_v4();
    let activated = service
        .activate_instance(
            actor,
            ActivateInstance::new(
                ReleaseCommandKey::derive("owner-history-activate", &[activation.as_bytes()]),
                InstanceActivationId::from_uuid(activation).unwrap(),
                prepared.instance_id(),
                prepared.revision_id(),
                1,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(activated.activated_version(), 2);
    let invocation = Uuid::new_v4();
    let command = InvokeInstance::new(
        ReleaseCommandKey::derive("owner-history-invoke", &[invocation.as_bytes()]),
        InstanceInvocationId::from_uuid(invocation).unwrap(),
        RunId::new(),
        CommandId::new(),
        prepared.instance_id(),
        prepared.revision_id(),
    )
    .unwrap();
    service.invoke_instance(actor, command).await.unwrap();
    let value: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM command_inbox WHERE command_id=$1")
            .bind(command.start_command_id().as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    let start: StartRun = serde_json::from_value(value).unwrap();
    assert_eq!(start.kind, RunKind::Invocation);
    assert_eq!(start.run_id, command.run_id());
    assert_eq!(start.instance_id, prepared.instance_id());
    assert_eq!(start.instance_revision_id, prepared.revision_id());
    assert!(!start.requires_state);
    assert!(start.attachment_id.is_none());
    let scope = RunCleanupHostId::new(
        config.provider_scope().namespace().into(),
        config.provider_scope().host_id().into(),
    )
    .unwrap();
    let runs = PgRunRepository::new(original.worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(scope.clone())
        .with_qualified_invocation_configuration(config.clone())
        .with_qualified_invocation_lease_runtime();
    (start, runs, scope)
}
