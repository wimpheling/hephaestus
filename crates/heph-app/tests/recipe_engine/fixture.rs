use std::{collections::BTreeMap, ops::Deref, sync::Arc};

use authz_postgres::PostgresMelangeAuthorizer;
use heph_runtime::VmProvider;
use hephaestus_app::{
    recipe_engine::{PreparedRecipeApplication, RecipeApplicationError, RecipeInstallation},
    recipe_instance_import::PreparedInstanceImporter,
    recipe_owned_volume::PreparedOwnedVolumeCreator,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentExecutionProfile, DeploymentKey, PlanningRequest, RecipePlanner,
};
use recipe_postgres::{PostgresExecutionPreparationRepository, PostgresPlanningCatalog};
use release_postgres::{InstanceExecutionConfiguration, PostgresInstanceExecutionService};
use sqlx::PgPool;

use crate::{instance_fixture, source_fixture, volume_fixture};

pub struct Fixture {
    pub pool: PgPool,
    pub base: volume_fixture::Fixture,
    pub engine: PreparedRecipeApplication,
}
impl Deref for Fixture {
    type Target = volume_fixture::Fixture;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
impl Fixture {
    pub async fn new(pool: &PgPool) -> Self {
        let base = volume_fixture::Fixture::new(pool).await;
        sqlx::raw_sql(include_str!(
            "/var/tmp/0114_qualified_instance_activation.corrected.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
        let engine = configured(
            &base,
            base.h.preparation.clone(),
            base.creator.clone(),
            base.h.importer(),
        );
        Self {
            pool: pool.clone(),
            base,
            engine,
        }
    }
    pub fn with_bad_volume_recorder(&self) -> PreparedRecipeApplication {
        configured(
            &self.base,
            self.h.preparation.clone(),
            self.bad_recorder(),
            self.h.importer(),
        )
    }
    pub fn with_bad_instance_recorder(&self) -> PreparedRecipeApplication {
        configured(
            &self.base,
            self.h.preparation.clone(),
            self.creator.clone(),
            self.h
                .configured(self.h.preparation.clone(), self.h.app.clone()),
        )
    }
    pub fn drifted(&self) -> PreparedRecipeApplication {
        let preparation = Arc::new(PostgresExecutionPreparationRepository::new(
            self.h.app.clone(),
            instance_fixture::platform(1024, "platform/drift"),
            DeploymentExecutionProfile::RuntimeNamedV1,
        ));
        configured(
            &self.base,
            preparation,
            self.creator.clone(),
            self.h.importer(),
        )
    }
    pub async fn resume(&self) -> Result<RecipeInstallation, RecipeApplicationError> {
        self.engine
            .resume(&self.h.actor, self.h.command, self.h.intent.id())
            .await
    }
    pub fn request(&self, key: &str) -> PlanningRequest {
        PlanningRequest::new(
            self.h.intent.project_id(),
            DeploymentKey::parse(key).unwrap(),
            self.h.intent.declaration_toml().as_bytes(),
            self.h.intent.inputs().clone(),
            BTreeMap::new(),
        )
        .unwrap()
    }
    pub async fn projection(&self) -> serde_json::Value {
        let rows:serde_json::Value = sqlx::query_scalar("SELECT jsonb_build_object('ledger',(SELECT jsonb_agg(to_jsonb(r) ORDER BY deployment_id,resource_name) FROM recipe_deployment_resources r),'results',(SELECT jsonb_agg(to_jsonb(r) ORDER BY command_id) FROM recipe_command_results r),'activations',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM agent_instance_execution_activations a),'instances',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM agent_instances i),'operations',(SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM volume_owned_provisioning_operations o),'proofs',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM recipe_effect_verifications p),'claims',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM recipe_effect_attempts c))")
            .fetch_one(&self.pool).await.unwrap();
        serde_json::json!([volume_fixture::snapshot(&self.pool).await, rows])
    }
    pub async fn withdraw_source(&self) {
        sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
            .bind(self.h.fixture.release)
            .execute(&self.pool)
            .await
            .unwrap();
    }
    pub fn fresh_request(&self) -> AuthenticatedIdentity {
        let mut identity = self.h.actor.clone();
        identity.request_id = identity_domain::RequestId::new();
        identity
    }
}
fn configured(
    base: &volume_fixture::Fixture,
    preparation: Arc<PostgresExecutionPreparationRepository>,
    volume: PreparedOwnedVolumeCreator,
    importer: PreparedInstanceImporter,
) -> PreparedRecipeApplication {
    // Scope labels are controlled backend comparison data, not VM ownership proof.
    let provider = vm_fake::FakeProvider::new_owned(base.owner.host().to_owned()).unwrap();
    let scope = provider.owner_scope().unwrap();
    let configuration = InstanceExecutionConfiguration::new(
        scope,
        base.root.path().to_str().unwrap().to_owned(),
        base.owner.namespace_id(),
        source_fixture::policy(512),
        "platform/import-v1".to_owned(),
    )
    .unwrap();
    PreparedRecipeApplication::new(
        Arc::new(RecipePlanner::new(PostgresPlanningCatalog::new(
            base.h.app.clone(),
            instance_fixture::platform(512, "platform/import-v1"),
        ))),
        base.h.repository.clone(),
        preparation,
        Arc::new(volume),
        Arc::new(importer),
        Arc::new(PostgresInstanceExecutionService::new(
            base.h.app.clone(),
            Arc::new(PostgresMelangeAuthorizer),
            configuration,
        )),
    )
}
