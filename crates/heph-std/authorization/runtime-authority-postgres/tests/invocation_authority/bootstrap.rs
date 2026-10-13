// Fixture-only adaptation of the frozen119 prerequisite bootstrap. All
// actor/claim/preparation/import/provider ports remain real. The only added
// catalogue inputs are optional non-Git requirements BEFORE publication/113.
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
    pub async fn new(pool: &PgPool, count: usize, sibling: bool) -> Self {
        Box::pin(Self::build(pool, count, sibling, None)).await
    }
    pub async fn with_git(pool: &PgPool, required: bool) -> Self {
        Box::pin(Self::build(pool, 0, false, Some(required))).await
    }
    async fn build(pool: &PgPool, count: usize, sibling: bool, git: Option<bool>) -> Self {
        fixtures::private_migrations::checkpoint(107)
            .run(pool)
            .await
            .unwrap();
        let historical_migrations = historical_migrations(pool).await;
        let fixture = published_source(pool, count, git).await;
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
        let declaration = super::graph::declaration(&fixture, count, sibling);
        let mut catalog = fixtures::catalog(&fixture);
        catalog.parameters = saved_parameters();
        catalog.capability_requirements = load_requirements(pool, fixture.release_agent).await;
        catalog.volume_slots =
            release_domain::effective_volume_slots(&super::graph::slots(count), false).unwrap();
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
            .instances()
            .iter()
            .find(|input| input.resource().as_str() == "sqlite")
            .unwrap()
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
    pub async fn assert_historical_migrations(&self, pool: &PgPool) {
        let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(
            after, self.historical_migrations,
            "genuine public107 checksums remain exact"
        );
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

async fn published_source(pool: &PgPool, count: usize, git: Option<bool>) -> support::Fixture {
    let mut fixture = seed::seed(pool).await;
    let mut contract = source_fixture::contract();
    contract["requires_state"] = serde_json::json!(false);
    contract["volume_slots"] = serde_json::to_value(super::graph::slots(count)).unwrap();
    let release = Uuid::new_v4();
    let agent = Uuid::new_v4();
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state) SELECT $1,repository_id,'activation-v1',source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'draft' FROM releases WHERE id=$2")
        .bind(release).bind(fixture.release).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state,parameter_schema) SELECT $1,$2,family_id,'activation','Activation',$3,$4,$5,$7 FROM release_agents WHERE id=$6")
        .bind(agent).bind(release).bind(&contract).bind(ContentHash::digest(&serde_json::to_vec(&contract).unwrap()).as_bytes().as_slice()).bind(false).bind(fixture.release_agent).bind(serde_json::to_value(saved_parameters()).unwrap()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO release_artifacts(id,release_id,path,kind,mode,content_hash,size_bytes,media_type,storage_key) VALUES($1,$2,'bin/sqlite','executable',493,$3,7,'application/octet-stream',$4)")
        .bind(Uuid::new_v4()).bind(release).bind([37_u8;32].as_slice()).bind(Uuid::new_v4())
        .execute(pool).await.unwrap();
    for slot in ["alpha", "zeta"] {
        let requirement = capability_domain::CapabilityRequirement::new(
            capability_domain::CapabilityRequirementId::new(),
            capability_domain::CapabilitySlotKey::parse(slot).unwrap(),
            capability_domain::CapabilityResourceKind::Project,
            [],
            [capability_domain::CapabilityOperation::Inspect],
            false,
        )
        .unwrap();
        sqlx::query("INSERT INTO release_capability_requirements(id,release_agent_id,slot_key,purpose,resource_kind,required_operations,optional_operations,slot_required,normalized_hash) VALUES($1,$2,$3,'Optional project inspection','project','{}',ARRAY['inspect'],false,$4)")
            .bind(requirement.id().as_uuid()).bind(agent).bind(slot)
            .bind(requirement.normalized_hash().as_bytes().as_slice())
            .execute(pool).await.unwrap();
    }
    if let Some(required) = git {
        let requirement = capability_domain::CapabilityRequirement::new(
            capability_domain::CapabilityRequirementId::new(),
            capability_domain::CapabilitySlotKey::parse("repository").unwrap(),
            capability_domain::CapabilityResourceKind::Repository,
            [capability_domain::CapabilityOperation::GitRead],
            [],
            required,
        )
        .unwrap();
        sqlx::query("INSERT INTO release_capability_requirements(id,release_agent_id,slot_key,purpose,resource_kind,required_operations,optional_operations,slot_required,normalized_hash) VALUES($1,$2,'repository','Git phase refusal','repository',ARRAY['git_read'],'{}',$3,$4)")
            .bind(requirement.id().as_uuid()).bind(agent).bind(required)
            .bind(requirement.normalized_hash().as_bytes().as_slice()).execute(pool).await.unwrap();
    }
    sqlx::query("UPDATE releases SET state='published',published_at=now() WHERE id=$1")
        .bind(release)
        .execute(pool)
        .await
        .unwrap();
    fixture.release = release;
    fixture.release_agent = agent;
    fixture
}

fn saved_parameters() -> Vec<release_domain::ParameterDeclaration> {
    vec![release_domain::ParameterDeclaration {
        name: release_domain::ParameterName::parse("greeting").unwrap(),
        value_type: release_domain::ParameterType::String {
            minimum_length: 1,
            maximum_length: 64,
        },
        required: true,
        default: Some(release_domain::ParameterValue::String("saved-é".to_owned())),
        sensitive: false,
    }]
}

async fn load_requirements(
    pool: &PgPool,
    agent: Uuid,
) -> Vec<capability_domain::CapabilityRequirement> {
    let rows:Vec<(serde_json::Value,Vec<u8>)>=sqlx::query_as("SELECT jsonb_build_object('id',id,'slot',slot_key,'resource_kind',resource_kind,'required_operations',required_operations,'optional_operations',optional_operations,'slot_required',slot_required),normalized_hash FROM release_capability_requirements WHERE release_agent_id=$1 ORDER BY slot_key COLLATE \"C\"")
  .bind(agent).fetch_all(pool).await.unwrap();
    rows.into_iter()
        .map(|(value, hash)| {
            let requirement: capability_domain::CapabilityRequirement =
                serde_json::from_value(value).unwrap();
            assert_eq!(requirement.normalized_hash().as_bytes().as_slice(), hash);
            requirement
        })
        .collect()
}

pub async fn required_git_planning_denied(pool: &PgPool) {
    fixtures::private_migrations::checkpoint(107)
        .run(pool)
        .await
        .unwrap();
    let fixture = published_source(pool, 0, Some(true)).await;
    let mut catalog = fixtures::catalog(&fixture);
    catalog.parameters = saved_parameters();
    catalog.volume_slots = vec![];
    catalog.capability_requirements = load_requirements(pool, fixture.release_agent).await;
    let parsed =
        recipe_domain::parse_recipe(super::graph::declaration(&fixture, 0, false).as_bytes())
            .unwrap();
    let error = parsed
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog])
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("unsupported_required_capability_slot"),
        "{error:?}"
    );
    let births: i64 = sqlx::query_scalar("SELECT count(*) FROM recipe_deployments")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        births, 0,
        "actual catalogue required-Git planning denies before preparation/claim/provider effects"
    );
}

async fn historical_migrations(pool: &PgPool) -> Vec<(i64, Vec<u8>)> {
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
    historical_migrations
}
