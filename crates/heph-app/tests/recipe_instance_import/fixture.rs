use std::{collections::BTreeMap, sync::Arc};

use authz_postgres::PostgresMelangeAuthorizer;
use hephaestus_app::recipe_instance_import::PreparedInstanceImporter;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    ActiveDeploymentEffectRepository, AttemptProvenance, CommandIdentity, DeploymentAttemptId,
    DeploymentExecutionPreparationRepository, DeploymentExecutionProfile, DeploymentIntent,
    EffectClaim, InstallDeployment, PlatformPolicyObservation, PreparedInstanceImport,
    ResourceAction, ResourceEffect,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository,
};
use release_domain::ContentHash;
use release_postgres::ReleaseService;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::{fixtures, schema, seed, source_fixture, support};

pub struct Harness {
    pub fixture: support::Fixture,
    pub actor: AuthenticatedIdentity,
    pub intent: DeploymentIntent,
    pub command: CommandIdentity,
    pub app: PgPool,
    pub worker: PgPool,
    pub repository: Arc<PostgresDeploymentRepository>,
    pub preparation: Arc<PostgresExecutionPreparationRepository>,
    pub release: Arc<ReleaseService>,
}

impl Harness {
    pub async fn new(pool: &PgPool, bound: bool) -> Self {
        Self::with_parameters(pool, bound, &[]).await
    }

    pub async fn with_parameters(
        pool: &PgPool,
        bound: bool,
        declarations: &[release_domain::ParameterDeclaration],
    ) -> Self {
        fixtures::private_migrations::checkpoint(107)
            .run(pool)
            .await
            .unwrap();
        let mut fixture = seed::seed(pool).await;
        let mut contract = source_fixture::contract();
        contract["requires_state"] = serde_json::json!(bound);
        let release = Uuid::new_v4();
        let agent = Uuid::new_v4();
        sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state)
          SELECT $1,repository_id,'instance-import-v1',source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'draft'
          FROM releases WHERE id=$2")
          .bind(release).bind(fixture.release).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state,parameter_schema)
          SELECT $1,$2,family_id,'prepared-import','Prepared import',$3,$4,$5,$7 FROM release_agents WHERE id=$6")
          .bind(agent).bind(release).bind(&contract).bind(ContentHash::digest(&serde_json::to_vec(&contract).unwrap()).as_bytes().as_slice())
          .bind(bound).bind(fixture.release_agent).bind(serde_json::to_value(declarations).unwrap()).execute(pool).await.unwrap();
        sqlx::query("UPDATE releases SET state='published',published_at=now() WHERE id=$1")
            .bind(release)
            .execute(pool)
            .await
            .unwrap();
        fixture.release = release;
        fixture.release_agent = agent;
        schema::checkpoint(113).run(pool).await.unwrap();
        let app = fixtures::app_pool(pool).await;
        let worker = PgPoolOptions::new()
            .max_connections(4)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap();
        let actor = support::identity(fixture.maintainer);
        let source = if bound {
            fixtures::source(&fixture, false)
        } else {
            format!(
                "contract_version=1\nrecipe_id=\"slotless\"\nrecipe_version=\"1.0.0\"\n[[resources]]\nkind=\"instance\"\nname=\"sqlite\"\nrelease={{release_id=\"{}\",release_agent_id=\"{}\"}}\nremoval=\"retain\"\n",
                fixture.release, fixture.release_agent
            )
        };
        let mut catalog = fixtures::catalog(&fixture);
        catalog.parameters = declarations.to_vec();
        catalog.volume_slots = release_domain::effective_volume_slots(&[], bound).unwrap();
        let intent = fixtures::build(
            &fixture,
            &source,
            recipe_application::DeploymentId::from_uuid(Uuid::new_v4()).unwrap(),
            "prepared-import",
            &BTreeMap::new(),
            &[catalog],
            false,
        );
        let repository = Arc::new(PostgresDeploymentRepository::new(app.clone()));
        let install = fixtures::install(&actor, intent.clone());
        let command = install.command;
        repository
            .admit_install(
                &actor,
                InstallDeployment {
                    command,
                    intent: intent.clone(),
                },
            )
            .await
            .unwrap();
        let preparation = Arc::new(PostgresExecutionPreparationRepository::new(
            app.clone(),
            platform(512, "platform/import-v1"),
            DeploymentExecutionProfile::RuntimeNamedV1,
        ));
        let release = Arc::new(ReleaseService::new(
            app.clone(),
            Arc::new(PostgresMelangeAuthorizer),
        ));
        Self {
            fixture,
            actor,
            intent,
            command,
            app,
            worker,
            repository,
            preparation,
            release,
        }
    }

    pub fn importer(&self) -> PreparedInstanceImporter {
        self.configured(self.preparation.clone(), self.worker.clone())
    }

    pub fn configured(
        &self,
        preparation: Arc<PostgresExecutionPreparationRepository>,
        worker: PgPool,
    ) -> PreparedInstanceImporter {
        PreparedInstanceImporter::new(
            preparation,
            self.repository.clone(),
            self.release.clone(),
            Arc::new(PostgresEffectVerificationRecorder::new(worker)),
        )
    }

    pub async fn prepared(&self) -> PreparedInstanceImport {
        self.preparation
            .prepare_execution(&self.actor, self.command, self.intent.id())
            .await
            .unwrap()
            .preparation
            .instances()[0]
            .clone()
    }

    pub async fn claim(&self) -> EffectClaim {
        self.prepared().await;
        let snapshot = self
            .repository
            .inspect(&self.actor, self.intent.id())
            .await
            .unwrap();
        self.repository
            .claim_resource_effect(
                &self.actor,
                ResourceEffect {
                    command: self.command,
                    deployment_id: self.intent.id(),
                    resource: fixtures::key("sqlite"),
                    expected_deployment_version: snapshot.version,
                    expected_resource_version: snapshot.resources[&fixtures::key("sqlite")].version,
                    action: ResourceAction::Create,
                    provenance: AttemptProvenance::new(
                        &self.actor,
                        DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                    )
                    .unwrap(),
                },
            )
            .await
            .unwrap()
    }

    pub async fn active(&self) -> Option<EffectClaim> {
        self.preparation
            .load_active_effect(
                &self.actor,
                self.command,
                self.intent.id(),
                &fixtures::key("sqlite"),
            )
            .await
            .unwrap()
    }

    pub fn prospective_slotless_import(&self) -> PreparedInstanceImport {
        self.prospective_slotless_preparation().instances()[0].clone()
    }

    pub fn prospective_slotless_preparation(
        &self,
    ) -> recipe_application::DeploymentExecutionPreparation {
        // Pure checked data for collision fixtures BEFORE protected113 admission.
        // This method does not pretend the value is an authoritative preparation.
        let mut contract = source_fixture::contract();
        contract["requires_state"] = serde_json::json!(false);
        let configured = platform(512, "platform/import-v1");
        let observation = recipe_application::SourceObservation::new(
            fixtures::catalog(&self.fixture).pin,
            ContentHash::digest(&serde_json::to_vec(&contract).unwrap()),
            contract["image_reference"].as_str().unwrap(),
            source_fixture::policy(128),
            configured.clone(),
        )
        .unwrap();
        recipe_application::DeploymentExecutionPreparation::new(
            &self.intent,
            self.command,
            DeploymentExecutionProfile::RuntimeNamedV1,
            configured,
            &[observation],
        )
        .unwrap()
    }
}

pub fn platform(memory: u32, version: &str) -> PlatformPolicyObservation {
    PlatformPolicyObservation::new(source_fixture::policy(memory), version.to_owned()).unwrap()
}

pub async fn state(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
      'claims',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_effect_attempts row),
      'progress',(SELECT jsonb_agg(to_jsonb(row) ORDER BY deployment_id,resource_name) FROM recipe_deployment_resources row),
      'mapping',(SELECT count(*) FROM recipe_owned_volume_preparations),
      'instances',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instances row),
      'revisions',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instance_revisions row),
      'commands',(SELECT jsonb_agg(to_jsonb(row) ORDER BY command_key) FROM release_command_inbox row),
      'grants',(SELECT count(*) FROM agent_instance_volume_mount_grants),
      'bindings',(SELECT count(*) FROM agent_instance_revision_volume_bindings),
      'creation_events',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instance_events row),
      'outbox',(SELECT count(*) FROM outbox),
      'product_outbox',(SELECT count(*) FROM product_event_outbox))")
      .fetch_one(pool).await.unwrap()
}
