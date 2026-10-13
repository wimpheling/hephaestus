use std::{collections::BTreeMap, sync::Arc};

use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId,
    DeploymentExecutionPreparationRepository, DeploymentExecutionProfile, DeploymentIntent,
    EffectClaim, EffectEvidence, EffectOutcome, PlatformPolicyObservation, PreparedInstanceImport,
    ResourceAction, ResourceEffect,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository, VerifiedOutcome,
};
use release_domain::ContentHash;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::{fixtures, preparation_schema, seed, source_fixture, support};

pub struct Bootstrap {
    pub historical_migrations: Vec<(i64, Vec<u8>)>,
    pub fixture: support::Fixture,
    pub actor: AuthenticatedIdentity,
    pub app: PgPool,
    pub worker: PgPool,
    pub intent: DeploymentIntent,
    pub command: CommandIdentity,
    pub recipe: Arc<PostgresDeploymentRepository>,
    pub input: PreparedInstanceImport,
}
impl Bootstrap {
    pub async fn new(pool: &PgPool, bound: bool) -> Self {
        fixtures::private_migrations::checkpoint(107)
            .run(pool)
            .await
            .unwrap();
        let historical_migrations: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(
            historical_migrations
                .iter()
                .filter(|(version, _)| *version == 107)
                .count(),
            1
        );
        let fixture = published_source(pool, bound).await;
        preparation_schema::checkpoint(113).run(pool).await.unwrap();
        let app = fixtures::app_pool(pool).await;
        let worker = PgPoolOptions::new()
            .max_connections(8)
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
        let declaration = if bound {
            fixtures::source(&fixture, false)
        } else {
            format!(
                "contract_version=1\nrecipe_id=\"activation\"\nrecipe_version=\"1.0.0\"\n[[resources]]\nkind=\"instance\"\nname=\"sqlite\"\nrelease={{release_id=\"{}\",release_agent_id=\"{}\"}}\nremoval=\"retain\"\n",
                fixture.release, fixture.release_agent
            )
        };
        let mut catalog = fixtures::catalog(&fixture);
        catalog.volume_slots = release_domain::effective_volume_slots(&[], bound).unwrap();
        let intent = fixtures::build(
            &fixture,
            &declaration,
            recipe_application::DeploymentId::from_uuid(Uuid::new_v4()).unwrap(),
            "activation",
            &BTreeMap::new(),
            &[catalog],
            false,
        );
        let recipe = Arc::new(PostgresDeploymentRepository::new(app.clone()));
        let install = fixtures::install(&actor, intent.clone());
        let command = install.command;
        recipe.admit_install(&actor, install).await.unwrap();
        let preparation = PostgresExecutionPreparationRepository::new(
            app.clone(),
            PlatformPolicyObservation::new(
                source_fixture::policy(512),
                "platform/activation-v1".to_owned(),
            )
            .unwrap(),
            DeploymentExecutionProfile::RuntimeNamedV1,
        );
        let input = preparation
            .prepare_execution(&actor, command, intent.id())
            .await
            .unwrap()
            .preparation
            .instances()[0]
            .clone();
        Self {
            historical_migrations,
            fixture,
            actor,
            app,
            worker,
            intent,
            command,
            recipe,
            input,
        }
    }
    pub async fn claim(&self, name: &str) -> EffectClaim {
        let snapshot = self
            .recipe
            .inspect(&self.actor, self.intent.id())
            .await
            .unwrap();
        let key = fixtures::key(name);
        self.recipe
            .claim_resource_effect(
                &self.actor,
                ResourceEffect {
                    command: self.command,
                    deployment_id: self.intent.id(),
                    resource: key.clone(),
                    expected_deployment_version: snapshot.version,
                    expected_resource_version: snapshot.resources[&key].version,
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
    pub async fn complete(&self, claim: &EffectClaim) {
        PostgresEffectVerificationRecorder::new(self.worker.clone())
            .record(claim, claim.provenance.attempt_id, VerifiedOutcome::Applied)
            .await
            .unwrap();
        self.recipe
            .complete_resource_effect(
                &self.actor,
                claim,
                EffectEvidence {
                    identity: claim.identity,
                    input_hash: claim.input_hash,
                    outcome: EffectOutcome::Applied,
                },
            )
            .await
            .unwrap();
    }
}

async fn published_source(pool: &PgPool, bound: bool) -> support::Fixture {
    let mut fixture = seed::seed(pool).await;
    let mut contract = source_fixture::contract();
    contract["requires_state"] = serde_json::json!(bound);
    let release = Uuid::new_v4();
    let agent = Uuid::new_v4();
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state) SELECT $1,repository_id,'activation-v1',source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'draft' FROM releases WHERE id=$2")
        .bind(release).bind(fixture.release).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) SELECT $1,$2,family_id,'activation','Activation',$3,$4,$5 FROM release_agents WHERE id=$6")
        .bind(agent).bind(release).bind(&contract).bind(ContentHash::digest(&serde_json::to_vec(&contract).unwrap()).as_bytes().as_slice()).bind(bound).bind(fixture.release_agent).execute(pool).await.unwrap();
    sqlx::query("UPDATE releases SET state='published',published_at=now() WHERE id=$1")
        .bind(release)
        .execute(pool)
        .await
        .unwrap();
    fixture.release = release;
    fixture.release_agent = agent;
    fixture
}
