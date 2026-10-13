//! Genuine import, activation, Invoke, browser Control admission and Review execution.

use control_plane_postgres::run::{ControlKind, ControlTarget, RequestControl, RunApplication};
use identity_domain::{AuthenticatedIdentity, RequestId};
use recipe_application::PreparedInstanceImport;
use release_domain::ReleaseCommandKey;
use release_postgres::PostgresInstanceExecutionService;
use release_service::{
    InstanceExecutionConfiguration, InstanceExecutionService, InstanceInvocationId, InvokeInstance,
};
use review_domain::ControlCommand;
use review_postgres::PostgresReviewRepository;
use review_service::{ControlOutcome, ReviewRepository};
use run_domain::{CancelRun, FreshRunExecutionProfile, RunCleanupHostId, StartRun};
use run_postgres::PgRunRepository;
use runtime_types::{CommandId, RunId};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::fixture::Fixture as Original;

pub(super) struct Fixture {
    pub original: Original,
    pub actor: AuthenticatedIdentity,
    pub config: InstanceExecutionConfiguration,
    pub start: StartRun,
    pub other: StartRun,
    pub runs: PgRunRepository,
    pub host: RunCleanupHostId,
}

impl Fixture {
    pub async fn new(pool: &PgPool) -> Self {
        let original = Original::new(pool).await;
        let config = crate::fixture::configuration(&original.scope, "test/v1");
        let (actor, prepared) = super::super::prepared::import(pool, &original, &config).await;
        let (start, runs, host) =
            super::super::birth::admit(pool, &original, &actor, &prepared, &config).await;
        let other = another_run(pool, &original, &actor, &prepared, &config).await;
        assert_eq!(other.instance_id, start.instance_id);
        assert_ne!(other.run_id, start.run_id);
        let runs = runs.with_qualified_cleanup_wake(host.clone());
        Self {
            original,
            actor,
            config,
            start,
            other,
            runs,
            host,
        }
    }

    pub fn repository(
        &self,
        pool: PgPool,
        config: InstanceExecutionConfiguration,
    ) -> PgRunRepository {
        PgRunRepository::new(pool)
            .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
            .with_vm_owner_scope(self.host.clone())
            .with_qualified_invocation_configuration(config)
            .with_qualified_invocation_lease_runtime()
            .with_qualified_cleanup_wake(self.host.clone())
    }

    pub async fn request(&self, pool: &PgPool, reason: &str) -> ControlCommand {
        // Distinct genuine user requests have distinct producer idempotency IDs.
        let actor = AuthenticatedIdentity::new(
            self.actor.user_id,
            &self.actor.issuer,
            &self.actor.subject,
            self.actor.verified_claims.clone(),
            RequestId::new(),
        );
        let root = tempfile::tempdir().unwrap();
        let requested = RunApplication::new(self.original.application.clone(), root.path().into())
            .request_control(
                &actor,
                RequestControl {
                    kind: ControlKind::Cancel,
                    repository_id: self.original.repository,
                    target: ControlTarget::Run(self.start.run_id.as_uuid()),
                    reason: reason.into(),
                },
            )
            .await
            .unwrap();
        drop(root);
        assert_eq!(requested.state, "pending");
        let payload: serde_json::Value = sqlx::query_scalar(
            "SELECT payload FROM outbox WHERE aggregate_type='control_request' AND aggregate_id=$1 AND event_type='control.requested'",
        ).bind(requested.id).fetch_one(pool).await.unwrap();
        let command: ControlCommand = serde_json::from_value(payload).unwrap();
        assert_eq!(command.command_id.as_uuid(), requested.id);
        command
    }

    pub async fn admit(&self, pool: &PgPool, command: &ControlCommand) -> CancelRun {
        let review = PostgresReviewRepository::new(self.original.application.clone());
        assert_eq!(
            review.execute_control(command).await.unwrap(),
            ControlOutcome::Completed
        );
        assert_eq!(
            review.execute_control(command).await.unwrap(),
            ControlOutcome::AlreadyCompleted
        );
        let payload: serde_json::Value = sqlx::query_scalar(
            "SELECT payload FROM outbox WHERE aggregate_type='run' AND aggregate_id=$1 AND event_type='run.cancel_requested' AND payload->>'command_id'=$2",
        ).bind(self.start.run_id.as_uuid()).bind(command.command_id.to_string())
            .fetch_one(pool).await.unwrap();
        let cancel: CancelRun = serde_json::from_value(payload).unwrap();
        assert_eq!(cancel.command_id.as_uuid(), command.command_id.as_uuid());
        assert_eq!(cancel.run_id, self.start.run_id);
        assert_eq!(cancel.reason, command.reason);
        cancel
    }
}

async fn another_run(
    pool: &PgPool,
    original: &Original,
    actor: &AuthenticatedIdentity,
    prepared: &PreparedInstanceImport,
    config: &InstanceExecutionConfiguration,
) -> StartRun {
    let id = Uuid::new_v4();
    let command = InvokeInstance::new(
        ReleaseCommandKey::derive("queued-cancel-other", &[id.as_bytes()]),
        InstanceInvocationId::from_uuid(id).unwrap(),
        RunId::new(),
        CommandId::new(),
        prepared.instance_id(),
        prepared.revision_id(),
    )
    .unwrap();
    PostgresInstanceExecutionService::new(
        original.application.clone(),
        Arc::new(authz_postgres::PostgresMelangeAuthorizer),
        config.clone(),
    )
    .with_qualified_invocation_admission()
    .invoke_instance(actor, command)
    .await
    .unwrap();
    let payload: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM command_inbox WHERE command_id=$1")
            .bind(command.start_command_id().as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    serde_json::from_value(payload).unwrap()
}
