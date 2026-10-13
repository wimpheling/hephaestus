use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use release_domain::ReleaseCommandKey;
use release_postgres::PostgresInstanceExecutionService;
use release_service::{InstanceExecutionService, InstanceInvocationId, InvokeInstance};
use runtime_types::{CommandId, RunId};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::activation_fixture::Harness;

pub struct Invocation {
    pub activation: Harness,
    pub command: InvokeInstance,
}
impl Invocation {
    pub async fn new(pool: &PgPool) -> Self {
        Self::build(pool, true).await
    }
    pub async fn unactivated(pool: &PgPool) -> Self {
        Self::build(pool, false).await
    }
    async fn build(pool: &PgPool, activate: bool) -> Self {
        let activation = Harness::new(pool).await;
        if activate {
            activation
                .service()
                .activate_instance(&activation.actor, activation.command)
                .await
                .unwrap();
        }
        let prior: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE version<=114 ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(
            prior.iter().filter(|(version, _)| *version == 114).count(),
            1
        );
        // Reuse the exact target-local114 chain, adding only the new115 proposal.
        let mut migrations = crate::preparation_schema::checkpoint(113);
        let mut all = migrations.iter().cloned().collect::<Vec<_>>();
        for (version, path) in [
            (
                114,
                include_str!(
                    "../../../../../../../migrations/0119_qualified_instance_activation.sql"
                ),
            ),
            (
                115,
                include_str!(
                    "../../../../../../../migrations/0120_qualified_instance_invocation.sql"
                ),
            ),
        ] {
            all.push(sqlx::migrate::Migration::new(
                version,
                "private_instance_execution".into(),
                sqlx::migrate::MigrationType::Simple,
                path.to_owned().into(),
                false,
            ));
        }
        migrations.migrations = all.into();
        migrations.run(pool).await.unwrap();
        let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE version<=114 ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(
            after, prior,
            "genuine114->115 preserves every historical migration checksum"
        );
        let id = Uuid::new_v5(
            &activation.command.activation_id().as_uuid(),
            b"invocation-fixture-v1",
        );
        let command = InvokeInstance::new(
            ReleaseCommandKey::derive("invoke_fixture", &[id.as_bytes()]),
            InstanceInvocationId::from_uuid(id).unwrap(),
            RunId::from_uuid(Uuid::new_v5(&id, b"run-v1")),
            CommandId::from_uuid(Uuid::new_v5(&id, b"start-v1")),
            activation.command.instance_id(),
            activation.command.expected_revision_id(),
        )
        .unwrap();
        Self {
            activation,
            command,
        }
    }
    pub fn service(&self) -> PostgresInstanceExecutionService {
        self.activation
            .service()
            .with_qualified_invocation_admission()
    }
    pub async fn rows(&self) -> serde_json::Value {
        sqlx::query_scalar("SELECT jsonb_build_object('instance',(SELECT to_jsonb(i) FROM agent_instances i WHERE id=$1),'requests',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM agent_instance_execution_invocations i),'links',(SELECT jsonb_agg(to_jsonb(i) ORDER BY invocation_id,permission) FROM agent_instance_execution_invocation_audits i),'runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM runs r WHERE instance_id=$1),'commands',(SELECT jsonb_agg(to_jsonb(c) ORDER BY command_key) FROM release_command_inbox c),'inbox',(SELECT jsonb_agg(to_jsonb(c) ORDER BY command_id) FROM command_inbox c),'outbox',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM outbox c),'events',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM application_events c),'products',(SELECT jsonb_agg(to_jsonb(c) ORDER BY event_id) FROM product_event_outbox c))")
            .bind(self.command.instance_id().as_uuid()).fetch_one(&self.activation.pool).await.unwrap()
    }
    pub async fn transaction(&self) -> Transaction<'_, Postgres> {
        begin_actor_transaction(&self.activation.app, &self.activation.actor)
            .await
            .unwrap()
    }
    pub async fn write(&self, tx: &mut Transaction<'_, Postgres>) {
        sqlx::query("SELECT * FROM write_qualified_instance_invocation($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(self.command.invocation_id().as_uuid())
            .bind(self.command.command_key().as_bytes().as_slice())
            .bind(self.command.run_id().as_uuid())
            .bind(self.command.start_command_id().as_uuid())
            .bind(self.command.instance_id().as_uuid())
            .bind(self.command.expected_revision_id().as_uuid())
            .bind(&self.activation.config_wire)
            .bind(AUTHORIZATION_MODEL_VERSION)
            .execute(&mut **tx)
            .await
            .unwrap();
    }
}
pub fn with_run(command: InvokeInstance, run: RunId) -> InvokeInstance {
    InvokeInstance::new(
        command.command_key(),
        command.invocation_id(),
        run,
        command.start_command_id(),
        command.instance_id(),
        command.expected_revision_id(),
    )
    .unwrap()
}
pub fn code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(std::borrow::Cow::into_owned)
}
