use crate::{bootstrap::Bootstrap, prepared_import, support};
use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentOperation, EffectClaim, PreparedInstanceSafetyTarget,
    RemoveDeployment,
};
use release_domain::{InstanceRemovalId, ReleaseCommandKey};
use release_postgres::{PostgresPreparedInstanceSafetyRepository, ReleaseService};
use release_service::RequestInstanceRemoval;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;

pub struct Fixture {
    pub base: Bootstrap,
    pub pool: PgPool,
    pub target: PreparedInstanceSafetyTarget,
}
impl Fixture {
    pub async fn new(pool: &PgPool, imported: bool, completed: bool) -> Self {
        let base = Bootstrap::new(pool, false).await;
        let claim = base.claim("sqlite").await;
        if imported {
            let returned =
                ReleaseService::new(base.app.clone(), Arc::new(PostgresMelangeAuthorizer))
                    .import_agent_with_volumes(&base.actor, prepared_import::command(&base.input))
                    .await
                    .unwrap();
            assert_eq!(returned, base.input.instance_id());
            if completed {
                base.complete(&claim).await;
            }
        }
        for file in [
            "/var/tmp/0108_legacy_vm_placement.sql",
            "/var/tmp/0114_qualified_instance_activation.corrected.sql",
            "/var/tmp/0116_owned_ready_retention.sql",
            "/var/tmp/0118_prepared_instance_safety.sql",
        ] {
            sqlx::raw_sql(&std::fs::read_to_string(file).unwrap())
                .execute(pool)
                .await
                .unwrap();
        }
        let historical: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(historical, base.historical_migrations);
        let target = PreparedInstanceSafetyTarget::new(
            claim,
            base.input.clone(),
            "opaque-provider-owner".into(),
            "actual-fixture-host".into(),
        )
        .unwrap();
        Self {
            base,
            pool: pool.clone(),
            target,
        }
    }
    pub fn service(&self) -> PostgresPreparedInstanceSafetyRepository {
        self.configured(self.base.app.clone(), self.base.worker.clone())
    }
    pub fn configured(
        &self,
        actor: PgPool,
        worker: PgPool,
    ) -> PostgresPreparedInstanceSafetyRepository {
        let scope = VmProviderOwnerScope::new(
            "opaque-provider-owner".to_owned(),
            "actual-fixture-host".to_owned(),
        )
        .unwrap();
        PostgresPreparedInstanceSafetyRepository::new(
            actor,
            worker,
            &scope,
            self.base.input.source().platform().clone(),
        )
    }
    pub async fn manager(&self) -> AuthenticatedIdentity {
        let manager = support::identity(self.base.fixture.outsider);
        sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
            .bind(self.base.fixture.consuming_project)
            .bind(manager.user_id.as_uuid())
            .execute(&self.pool)
            .await
            .unwrap();
        manager
    }
    pub async fn admit_remove(&self, manager: &AuthenticatedIdentity) -> CommandIdentity {
        let snapshot = self
            .base
            .recipe
            .inspect(manager, self.base.intent.id())
            .await
            .unwrap();
        let command = CommandIdentity::from_identity(manager, DeploymentOperation::Remove).unwrap();
        self.base
            .recipe
            .admit_remove(
                manager,
                RemoveDeployment {
                    command,
                    deployment_id: self.base.intent.id(),
                    expected_version: snapshot.version,
                },
            )
            .await
            .unwrap();
        command
    }
    pub async fn close(&self, manager: &AuthenticatedIdentity) {
        let version: i64 = sqlx::query_scalar("SELECT version FROM agent_instances WHERE id=$1")
            .bind(self.base.input.instance_id().as_uuid())
            .fetch_one(&self.pool)
            .await
            .unwrap();
        ReleaseService::new(self.base.app.clone(), Arc::new(PostgresMelangeAuthorizer))
            .request_instance_removal(
                manager,
                RequestInstanceRemoval {
                    command_key: ReleaseCommandKey::derive(
                        "observer_remove",
                        &[self.base.input.instance_id().as_uuid().as_bytes()],
                    ),
                    instance_id: self.base.input.instance_id(),
                    removal_id: InstanceRemovalId::from_uuid(Uuid::new_v4()),
                    expected_version: u64::try_from(version).unwrap(),
                },
            )
            .await
            .unwrap();
    }
    pub async fn snapshot(&self) -> serde_json::Value {
        sqlx::query_scalar("SELECT jsonb_build_object('instances',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM agent_instances i),'inbox',(SELECT jsonb_agg(to_jsonb(i) ORDER BY command_key) FROM release_command_inbox i),'claims',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM recipe_effect_attempts a),'proofs',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM recipe_effect_verifications p),'transitions',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM recipe_execution_transitions t),'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM application_events e),'removal',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM agent_instance_removal_requests r),'leases',(SELECT jsonb_agg(to_jsonb(l) ORDER BY id) FROM agent_instance_volume_leases l))")
            .fetch_one(&self.pool).await.unwrap()
    }
    pub fn changed(&self, claim: EffectClaim) -> PreparedInstanceSafetyTarget {
        PreparedInstanceSafetyTarget::new(
            claim,
            self.base.input.clone(),
            "opaque-provider-owner".into(),
            "actual-fixture-host".into(),
        )
        .unwrap()
    }
}
