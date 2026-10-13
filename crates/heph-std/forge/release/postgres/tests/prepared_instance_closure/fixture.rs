use crate::original_fixture::Fixture;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, PreparedInstanceClosureRequest,
    PreparedInstanceSafetyRepository, ResourceAction, ResourceEffect,
};
use sqlx::PgPool;
use uuid::Uuid;

pub struct Closure {
    pub original: Fixture,
    pub manager: AuthenticatedIdentity,
    pub command: CommandIdentity,
}
impl Closure {
    pub async fn new(pool: &PgPool, completed: bool) -> Self {
        let original = Fixture::new(pool, true, completed).await;
        sqlx::raw_sql(include_str!(
            "../../../../../../../migrations/0128_prepared_instance_closure.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
        let catalog: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(catalog, original.base.historical_migrations);
        let manager = original.manager().await;
        let command = original.admit_remove(&manager).await;
        Self {
            original,
            manager,
            command,
        }
    }
    pub async fn request(&self) -> PreparedInstanceClosureRequest {
        let base = &self.original.base;
        let loaded = self
            .original
            .service()
            .load_original_target(
                &self.manager,
                self.command,
                &base.intent,
                base.input.resource(),
            )
            .await
            .unwrap();
        assert_eq!(loaded, self.original.target);
        let snapshot = base
            .recipe
            .inspect(&self.manager, base.intent.id())
            .await
            .unwrap();
        let drain = base
            .recipe
            .claim_resource_effect(
                &self.manager,
                ResourceEffect {
                    command: self.command,
                    deployment_id: base.intent.id(),
                    resource: base.input.resource().clone(),
                    expected_deployment_version: snapshot.version,
                    expected_resource_version: snapshot.resources[base.input.resource()].version,
                    action: ResourceAction::Drain,
                    provenance: AttemptProvenance::new(
                        &self.manager,
                        DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                    )
                    .unwrap(),
                },
            )
            .await
            .unwrap();
        let version: i64 = sqlx::query_scalar("SELECT version FROM agent_instances WHERE id=$1")
            .bind(base.input.instance_id().as_uuid())
            .fetch_one(&self.original.pool)
            .await
            .unwrap();
        PreparedInstanceClosureRequest::new(
            base.intent.clone(),
            loaded,
            drain,
            u64::try_from(version).unwrap(),
        )
        .unwrap()
    }
    pub async fn snapshot(&self) -> serde_json::Value {
        let original = self.original.snapshot().await;
        let detail: serde_json::Value = sqlx::query_scalar("SELECT jsonb_build_object('base',(SELECT jsonb_build_object('instances',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM agent_instances i),'removal',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM agent_instance_removal_requests i),'inbox',(SELECT jsonb_agg(to_jsonb(i) ORDER BY command_key) FROM release_command_inbox i),'claims',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM recipe_effect_attempts i),'events',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM application_events i))),'outbox',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM outbox i),'product',(SELECT jsonb_agg(to_jsonb(i) ORDER BY event_id) FROM product_event_outbox i),'audits',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM authorization_audit_events i))")
            .fetch_one(&self.original.pool).await.unwrap();
        serde_json::json!({"original":original,"detail":detail})
    }
}
