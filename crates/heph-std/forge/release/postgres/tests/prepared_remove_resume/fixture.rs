use crate::{closure_fixture::Closure, support};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentExecutionProfile, DeploymentOperation,
    PreparedInstanceClosureRepository, PreparedInstanceClosureRequest, RemoveDeployment,
    RemoveResumeConfiguration,
};
use recipe_postgres::{PostgresDeploymentRepository, PostgresExecutionPreparationRepository};
use sqlx::PgPool;

pub struct Fixture {
    pub closure: Closure,
    pub request: PreparedInstanceClosureRequest,
    pub manager: AuthenticatedIdentity,
    pub remove: RemoveDeployment,
    pub config: RemoveResumeConfiguration,
}
impl Fixture {
    pub async fn new(pool: &PgPool) -> Self {
        let closure = Closure::new(pool, true).await;
        let request = closure.request().await;
        closure
            .original
            .service()
            .close_for_remove(&closure.manager, &request)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../../../../../../migrations/0129_qualified_remove_resume.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
        let base = &closure.original.base;
        let manager = support::identity(base.fixture.member);
        sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
            .bind(base.fixture.consuming_project)
            .bind(manager.user_id.as_uuid())
            .execute(pool)
            .await
            .unwrap();
        let snapshot = base
            .recipe
            .inspect(&manager, base.intent.id())
            .await
            .unwrap();
        let remove = RemoveDeployment {
            command: CommandIdentity::from_identity(&manager, DeploymentOperation::Remove).unwrap(),
            deployment_id: base.intent.id(),
            expected_version: snapshot.version,
        };
        let config = RemoveResumeConfiguration::new(
            closure.original.target.provider_namespace().into(),
            closure.original.target.host_id().into(),
            base.input.source().platform().clone(),
            DeploymentExecutionProfile::RuntimeNamedV1,
        )
        .unwrap();
        Self {
            closure,
            request,
            manager,
            remove,
            config,
        }
    }
    pub fn repository(&self) -> PostgresDeploymentRepository {
        PostgresDeploymentRepository::new(self.closure.original.base.app.clone())
            .with_remove_resume(self.config.clone())
    }
    pub fn preparation(&self) -> PostgresExecutionPreparationRepository {
        PostgresExecutionPreparationRepository::new(
            self.closure.original.base.app.clone(),
            self.config.platform().clone(),
            self.config.profile(),
        )
        .with_remove_resume(self.config.clone())
        .unwrap()
    }
    pub async fn withdraw_original(&self) {
        sqlx::query("UPDATE users SET status='disabled' WHERE id=$1")
            .bind(self.closure.manager.user_id.as_uuid())
            .execute(&self.closure.original.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
            .bind(self.closure.original.base.fixture.release)
            .execute(&self.closure.original.pool)
            .await
            .unwrap();
    }
    pub async fn snapshot(&self) -> serde_json::Value {
        let shared = self.closure.snapshot().await;
        let records:serde_json::Value=sqlx::query_scalar("SELECT jsonb_build_object('admissions',(SELECT jsonb_agg(to_jsonb(a) ORDER BY command_id) FROM recipe_remove_resume_admissions a),
            'commands',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM recipe_deployment_commands c),
            'resources',(SELECT jsonb_agg(to_jsonb(r) ORDER BY deployment_id,resource_name) FROM recipe_deployment_resources r),
            'deployments',(SELECT jsonb_agg(to_jsonb(d) ORDER BY id) FROM recipe_deployments d))")
            .fetch_one(&self.closure.original.pool).await.unwrap();
        serde_json::json!({"shared":shared,"records":records})
    }
    pub async fn execution_snapshot(&self) -> serde_json::Value {
        // Fresh decision audits on successful replay are intentional; compare
        // complete execution/event rows independently from authorization audits.
        sqlx::query_scalar("SELECT jsonb_build_object('resources',(SELECT jsonb_agg(to_jsonb(r) ORDER BY deployment_id,resource_name) FROM recipe_deployment_resources r),
            'transitions',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM recipe_execution_transitions t),
            'proofs',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM recipe_effect_verifications p),
            'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM application_events e),
            'outbox',(SELECT jsonb_agg(to_jsonb(o) ORDER BY event_id) FROM product_event_outbox o),
            'admissions',(SELECT jsonb_agg(to_jsonb(a) ORDER BY command_id) FROM recipe_remove_resume_admissions a))")
            .fetch_one(&self.closure.original.pool).await.unwrap()
    }
    pub async fn preserved(&self) -> serde_json::Value {
        sqlx::query_scalar("SELECT jsonb_build_object('instance',(SELECT to_jsonb(i) FROM agent_instances i WHERE id=$1),
            'closure',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM agent_instance_removal_requests r WHERE instance_id=$1),
            'release_inbox',(SELECT jsonb_agg(to_jsonb(c) ORDER BY command_key) FROM release_command_inbox c WHERE aggregate_id=$1),
            'claims',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM recipe_effect_attempts a WHERE deployment_id=$2),
            'version',(SELECT version FROM recipe_deployments WHERE id=$2),'leases',(SELECT jsonb_agg(to_jsonb(l) ORDER BY id) FROM agent_instance_volume_leases l))")
            .bind(self.closure.original.base.input.instance_id().as_uuid()).bind(self.remove.deployment_id.as_uuid())
            .fetch_one(&self.closure.original.pool).await.unwrap()
    }
}
