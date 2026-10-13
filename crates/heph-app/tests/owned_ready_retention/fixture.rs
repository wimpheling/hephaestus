use crate::{birth, fixtures, support};
use heph_runtime::VmProvider;
use hephaestus_app::recipe::ready_retention::ReadyRetentionCoordinator;
use identity_domain::{AuthenticatedIdentity, UserId};
use recipe_application::{
    DeploymentAttemptId, DeploymentLifecycle, EffectClaim, OwnedReadyRemoveResume,
    OwnedReadyRetentionRepository, RetainOwnedReady,
};
use recipe_postgres::PostgresOwnedReadyRetentionRepository;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
use vm_fake::FakeProvider;
use volume_postgres::PostgresReadyRetentionRepository;
use volume_trait::OwnedProvisioningContext;

pub struct Fixture {
    pub pool: PgPool,
    pub birth: birth::Fixture,
    pub target: EffectClaim,
    pub ready: OwnedProvisioningContext,
    pub actor: Arc<PostgresOwnedReadyRetentionRepository>,
    pub worker: Arc<PostgresReadyRetentionRepository>,
    pub provider: Arc<FakeProvider>,
}
impl Fixture {
    pub async fn new(pool: &PgPool, completed: bool) -> Self {
        let birth = birth::Fixture::new(pool).await;
        // Target-local successors only; no changes to published bookkeeping.
        sqlx::raw_sql(include_str!(
            "../../../../migrations/0112_legacy_vm_placement.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../../../../migrations/0121_owned_ready_retention.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
        let (target, request) = birth.request().await;
        let ready = birth
            .local
            .provision_owned(&birth.h.actor, &request)
            .await
            .unwrap();
        if completed {
            let custody = birth.local.verify_owned_ready(&ready.claim).await.unwrap();
            birth::finish_held(&birth, target.clone(), custody).await;
        }
        let provider = Arc::new(FakeProvider::new_owned("retention-fixture-host".into()).unwrap());
        let scope = provider.owner_scope().unwrap();
        let actor = Arc::new(
            PostgresOwnedReadyRetentionRepository::new(
                birth.h.app.clone(),
                birth.h.preparation.as_ref().clone(),
                scope.namespace().to_owned(),
                scope.host_id().to_owned(),
            )
            .unwrap(),
        );
        let worker = Arc::new(
            PostgresReadyRetentionRepository::new(
                birth.h.worker.clone(),
                scope.namespace().to_owned(),
                scope.host_id().to_owned(),
            )
            .unwrap(),
        );
        Self {
            pool: pool.clone(),
            birth,
            target,
            ready,
            actor,
            worker,
            provider,
        }
    }
    pub fn coordinator(&self) -> ReadyRetentionCoordinator {
        ReadyRetentionCoordinator::new(
            self.actor.clone(),
            self.worker.clone(),
            Arc::new(self.birth.local.clone()),
            self.provider.clone(),
        )
    }
    pub async fn remove(&self, user: UserId) -> (AuthenticatedIdentity, RetainOwnedReady) {
        let identity = support::identity(user);
        let snapshot = self
            .birth
            .h
            .repository
            .inspect(&identity, self.target.deployment_id)
            .await
            .unwrap();
        let removal = fixtures::remove(&identity, self.target.deployment_id, snapshot.version);
        let command = removal.command;
        let admitted = if snapshot.lifecycle == DeploymentLifecycle::Removing {
            match self
                .actor
                .resume_owned_ready_remove(&identity, &removal, &self.target)
                .await
                .unwrap()
            {
                OwnedReadyRemoveResume::Admitted(admission) => *admission,
                OwnedReadyRemoveResume::Retained(_) => {
                    panic!("test requested new effect on already retained target")
                }
            }
        } else {
            self.birth
                .h
                .repository
                .admit_remove(&identity, removal)
                .await
                .unwrap()
        };
        let request = RetainOwnedReady {
            command,
            target: self.target.clone(),
            operation_id: DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
            expected_deployment_version: admitted.snapshot.version,
            expected_resource_version: admitted.snapshot.resources[&fixtures::key("data")].version,
        };
        (identity, request)
    }
    pub async fn manager(&self, user: UserId) {
        sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(self.birth.h.fixture.consuming_project).bind(user.as_uuid()).execute(&self.pool).await.unwrap();
    }
    pub async fn withdraw_installer(&self) {
        sqlx::query("DELETE FROM project_maintainers WHERE user_id=$1")
            .bind(self.birth.h.actor.user_id.as_uuid())
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE releases SET state='revoked',revoked_at=COALESCE(revoked_at,now()) WHERE id=$1 AND state='published'")
            .bind(self.birth.h.fixture.release).execute(&self.pool).await.unwrap();
    }
    pub async fn original(&self) -> serde_json::Value {
        sqlx::query_scalar("SELECT jsonb_build_object('claim',(SELECT claim_json FROM recipe_effect_attempts WHERE id=$1),'install_transitions',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM recipe_execution_transitions t WHERE command_id=$2),'verifications',(SELECT jsonb_agg(to_jsonb(v) ORDER BY id) FROM recipe_effect_verifications v WHERE attempt_id=$1),'seal',(SELECT to_jsonb(s) FROM volume_creation_seals s WHERE volume_id=$3),'purposes',(SELECT to_jsonb(p) FROM volume_owned_backing_purposes p WHERE volume_id=$3),'operations',(SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM volume_owned_provisioning_operations o WHERE volume_id=$3),'observations',(SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM volume_owned_backing_observations o WHERE volume_id=$3))")
            .bind(self.target.provenance.attempt_id.as_uuid()).bind(self.target.command.id().as_uuid())
            .bind(self.ready.claim.purpose.receipt().intent.registration().id().as_uuid())
            .fetch_one(&self.pool).await.unwrap()
    }
    pub async fn snapshot(&self) -> serde_json::Value {
        let original = birth::snapshot(&self.pool).await;
        let disposition:serde_json::Value=sqlx::query_scalar("SELECT jsonb_build_object('closures',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM volume_owned_ready_retention_closures c),'facts',(SELECT jsonb_agg(to_jsonb(f) ORDER BY id) FROM volume_owned_ready_retention_facts f),'retained',(SELECT jsonb_agg(to_jsonb(r) ORDER BY closure_id) FROM recipe_owned_ready_retentions r),'reopened',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM recipe_owned_ready_reopenings r),'remove_admissions',(SELECT jsonb_agg(to_jsonb(r) ORDER BY command_id) FROM recipe_owned_ready_remove_admissions r))")
            .fetch_one(&self.pool).await.unwrap();
        serde_json::json!([original, disposition])
    }
}
