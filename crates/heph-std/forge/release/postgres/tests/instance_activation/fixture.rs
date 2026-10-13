use std::sync::Arc;

use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{DeploymentIntent, EffectClaim};
use recipe_postgres::PostgresDeploymentRepository;
use release_domain::{AgentInstanceRevisionId, ReleaseCommandKey};
use release_postgres::{
    InstanceExecutionConfiguration, PostgresInstanceExecutionService, ReleaseService,
};
use release_service::{ActivateInstance, InstanceActivationId};
use sqlx::PgPool;
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;
use volume_domain::VolumeRootNamespaceId;

use super::{bootstrap::Bootstrap, owned_birth::OwnedBirth};
use crate::{preparation_schema, source_fixture, support};

pub struct Harness {
    pub pool: PgPool,
    pub app: PgPool,
    pub worker: PgPool,
    pub pending: Option<EffectClaim>,
    pub actor: AuthenticatedIdentity,
    pub fixture: support::Fixture,
    pub intent: DeploymentIntent,
    pub recipe: Arc<PostgresDeploymentRepository>,
    pub command: ActivateInstance,
    pub config: InstanceExecutionConfiguration,
    pub config_wire: Vec<u8>,
    pub physical: Option<OwnedBirth>,
}
impl Harness {
    pub async fn new(pool: &PgPool) -> Self {
        Self::build(pool, false, true).await
    }
    pub async fn bound(pool: &PgPool) -> Self {
        Self::build(pool, true, true).await
    }
    pub async fn pending(pool: &PgPool) -> Self {
        Self::build(pool, false, false).await
    }
    async fn build(pool: &PgPool, bound: bool, complete: bool) -> Self {
        let mut bootstrap = Bootstrap::new(pool, bound).await;
        let physical = if bound {
            Some(OwnedBirth::new(pool, &bootstrap).await)
        } else {
            None
        };
        let claim = bootstrap.claim("sqlite").await;
        let returned =
            ReleaseService::new(bootstrap.app.clone(), Arc::new(PostgresMelangeAuthorizer))
                .import_agent_with_volumes(
                    &bootstrap.actor,
                    super::prepared_import::command(&bootstrap.input),
                )
                .await
                .unwrap();
        assert_eq!(
            returned,
            bootstrap.input.instance_id(),
            "real107 successful typed import return"
        );
        if complete {
            bootstrap.complete(&claim).await;
        }
        let (root, namespace) = physical.as_ref().map_or_else(
            || ("/tmp".to_owned(), Uuid::new_v4()),
            |birth| {
                (
                    birth.root.path().to_str().unwrap().to_owned(),
                    birth.owner.namespace_id().as_uuid(),
                )
            },
        );
        let config = configuration(
            "vm-owner-fixture",
            &root,
            namespace,
            "platform/activation-v1",
        );
        let config_wire=serde_json::to_vec(&serde_json::json!({"version":1,"profile":"runtime_named_v1","vm":{"namespace":"vm-owner-fixture","host_id":"fixture-host"},"volume":{"namespace":namespace,"host_id":"fixture-host","root":root},"platform":{"policy":source_fixture::policy(512),"version":"platform/activation-v1"}})).unwrap();
        let command = ActivateInstance::new(
            ReleaseCommandKey::derive("activate_fixture", &[returned.as_uuid().as_bytes()]),
            InstanceActivationId::from_uuid(Uuid::new_v4()).unwrap(),
            returned,
            bootstrap.input.revision_id(),
            1,
        )
        .unwrap();
        let mut migrations = preparation_schema::checkpoint(113);
        let mut all = migrations.iter().cloned().collect::<Vec<_>>();
        all.push(sqlx::migrate::Migration::new(
            114,
            "private_qualified_activation".into(),
            sqlx::migrate::MigrationType::Simple,
            std::fs::read_to_string("/var/tmp/0114_qualified_instance_activation.corrected.sql")
                .unwrap()
                .into(),
            false,
        ));
        migrations.migrations = all.into();
        migrations.run(pool).await.unwrap();
        let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(
            after, bootstrap.historical_migrations,
            "real public107 migration checksums remain unchanged through private114"
        );
        bootstrap.fixture.instance = returned.as_uuid();
        Self {
            pool: pool.clone(),
            app: bootstrap.app,
            worker: bootstrap.worker,
            pending: if complete { None } else { Some(claim) },
            actor: bootstrap.actor,
            fixture: bootstrap.fixture,
            intent: bootstrap.intent,
            recipe: bootstrap.recipe,
            command,
            config,
            config_wire,
            physical,
        }
    }
    pub fn service(&self) -> PostgresInstanceExecutionService {
        self.configured(self.config.clone())
    }
    pub fn configured(
        &self,
        config: InstanceExecutionConfiguration,
    ) -> PostgresInstanceExecutionService {
        PostgresInstanceExecutionService::new(
            self.app.clone(),
            Arc::new(PostgresMelangeAuthorizer),
            config,
        )
    }
    pub async fn rows(&self) -> serde_json::Value {
        sqlx::query_scalar("SELECT jsonb_build_object('instance',(SELECT to_jsonb(i) FROM agent_instances i WHERE id=$1),'activation',(SELECT jsonb_agg(to_jsonb(a)) FROM agent_instance_execution_activations a),'commands',(SELECT jsonb_agg(to_jsonb(c) ORDER BY command_key) FROM release_command_inbox c),'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM agent_instance_events e WHERE instance_id=$1),'outbox',(SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM outbox o WHERE aggregate_id=$1),'runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM runs r WHERE instance_id=$1))")
            .bind(self.command.instance_id().as_uuid()).fetch_one(&self.pool).await.unwrap()
    }
    pub async fn permission(
        &self,
        identity: &AuthenticatedIdentity,
        permission: Permission,
        object: ObjectRef,
    ) -> bool {
        let mut tx = begin_actor_transaction(&self.app, identity).await.unwrap();
        PostgresMelangeAuthorizer
            .check(&mut tx, Subject::User(identity.user_id), permission, object)
            .await
            .unwrap()
            == AuthorizationDecision::Allow
    }
    pub async fn can_use(&self, identity: &AuthenticatedIdentity) -> bool {
        self.permission(
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, self.fixture.release_agent),
        )
        .await
    }
}
pub fn configuration(
    namespace: &str,
    root: &str,
    volume_namespace: Uuid,
    version: &str,
) -> InstanceExecutionConfiguration {
    InstanceExecutionConfiguration::new(
        VmProviderOwnerScope::new(namespace.to_owned(), "fixture-host".to_owned()).unwrap(),
        root.to_owned(),
        VolumeRootNamespaceId::from_uuid(volume_namespace).unwrap(),
        source_fixture::policy(512),
        version.to_owned(),
    )
    .unwrap()
}
pub fn changed_revision(command: ActivateInstance) -> ActivateInstance {
    ActivateInstance::new(
        command.command_key(),
        command.activation_id(),
        command.instance_id(),
        AgentInstanceRevisionId::from_uuid(Uuid::new_v4()),
        1,
    )
    .unwrap()
}
pub fn another_command(command: ActivateInstance) -> ActivateInstance {
    ActivateInstance::new(
        ReleaseCommandKey::derive(
            "another_activation",
            &[command.instance_id().as_uuid().as_bytes()],
        ),
        InstanceActivationId::from_uuid(Uuid::new_v4()).unwrap(),
        command.instance_id(),
        command.expected_revision_id(),
        1,
    )
    .unwrap()
}
