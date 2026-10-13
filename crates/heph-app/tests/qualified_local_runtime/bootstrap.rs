//! Actual PG planning, prepared owned births, import and Invocation producers.

use std::{collections::BTreeMap, fmt::Write as _, sync::Arc};

use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId,
    DeploymentExecutionPreparationRepository, DeploymentExecutionProfile, DeploymentIntent,
    DeploymentOperation, EffectClaim, EffectEvidence, EffectOutcome, InstallDeployment,
    PlanningRequest, PlatformPolicyObservation, PreparedInstanceImport, RecipePlan, RecipePlanner,
    ResourceAction, ResourceEffect,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository, PostgresPlanningCatalog, VerifiedOutcome,
};
use run_domain::{RunCleanupHostId, StartRun};
use run_postgres::PgRunRepository;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;
use vm_trait::VmId;

use super::{
    owned_births::OwnedBirths,
    publication::{PublishedVariant, StoppedPublicationBackend},
};
use crate::fixtures;

#[path = "bootstrap_phases.rs"]
mod phases;

// Both variants are admitted only through genuinely stopped inputs.
pub async fn invoke_pair(stopped: [StoppedPublicationBackend; 2]) -> [InvocationBirth; 2] {
    phases::invoke_pair(stopped).await
}

struct PlannedVariant {
    stopped: StoppedPublicationBackend,
    app: PgPool,
    worker: PgPool,
    resolved: RecipePlan,
    historical_migrations: Vec<(i64, Vec<u8>)>,
}

// Planning borrows a token backed by the actual completed owned drain.
async fn plan(stopped: StoppedPublicationBackend) -> PlannedVariant {
    stopped
        .validate_handoff()
        .expect("same retained post-drain backend");
    let published = stopped.publication();
    published.validate_identity();
    crate::authority_schema::verify_inputs();
    let historical_migrations = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
    )
    .fetch_all(&published.pool)
    .await
    .unwrap();
    crate::preparation_schema::checkpoint(113)
        .run(&published.pool)
        .await
        .unwrap();
    let app = fixtures::app_pool(&published.pool).await;
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
        .connect_with(published.pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let configuration = &published.configuration;
    let platform = PlatformPolicyObservation::new(
        configuration.platform_policy().clone(),
        configuration.platform_version().to_owned(),
    )
    .unwrap();
    let request = PlanningRequest::new(
        published.project_id(),
        recipe_application::DeploymentKey::parse(published.variant.key()).unwrap(),
        declaration(published).as_bytes(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();
    let resolved = RecipePlanner::new(PostgresPlanningCatalog::new(app.clone(), platform))
        .plan(&published.actor, &request)
        .await
        .unwrap();
    PlannedVariant {
        stopped,
        app,
        worker,
        resolved,
        historical_migrations,
    }
}

fn declaration(published: &PublishedVariant) -> String {
    let mut source = format!(
        "contract_version=1\nrecipe_id=\"{}\"\nrecipe_version=\"1.0.0\"\n[[inputs]]\nname=\"capacity\"\ndefault=16777216\nvalue_type={{type=\"integer\",minimum=16777216,maximum=1073741824}}\n",
        published.variant.key(),
    );
    let slots = published.variant.slots();
    for index in 0..slots.len() {
        writeln!(source, "[[resources]]\nkind=\"volume\"\nname=\"data{index:02}\"\nsource={{type=\"created\",capacity_bytes={{source=\"input\",name=\"capacity\"}}}}\nremoval=\"retain\"")
            .expect("writing to a String cannot fail");
    }
    let bindings = slots.iter().enumerate().map(|(index, slot)| {
        format!("{{slot=\"{}\",resource=\"data{index:02}\",guest_path=\"{}\",access_mode=\"read_write\"}}", slot.slot(), slot.guest_path().as_str())
    }).collect::<Vec<_>>().join(",");
    writeln!(source, "[[resources]]\nkind=\"instance\"\nname=\"sqlite\"\nrelease={{release_id=\"{}\",release_agent_id=\"{}\"}}\nvolume_bindings=[{bindings}]\nparameters={{operation={{source=\"literal\",value=\"inspect\"}},key={{source=\"literal\",value=\"saved-é\"}},value={{source=\"literal\",value=\"saved value\"}}}}\nremoval=\"retain\"", published.published.release_id, published.published.release_agent_id)
        .expect("writing to a String cannot fail");
    source
}

pub struct Bootstrap {
    pub published: Arc<PublishedVariant>,
    pub actor: AuthenticatedIdentity,
    pub app: PgPool,
    pub worker: PgPool,
    pub intent: DeploymentIntent,
    pub command: CommandIdentity,
    pub recipe: Arc<PostgresDeploymentRepository>,
    pub input: PreparedInstanceImport,
    historical_migrations: Vec<(i64, Vec<u8>)>,
    stopped: StoppedPublicationBackend,
}
impl Bootstrap {
    async fn new(planned: PlannedVariant) -> Self {
        planned
            .stopped
            .validate_handoff()
            .expect("same sealed Invocation capability");
        let published = planned.stopped.publication().clone();
        published.validate_identity();
        let actor = published.actor.clone();
        let intent = planned.resolved.intent().clone();
        let command = CommandIdentity::from_identity(&actor, DeploymentOperation::Install).unwrap();
        let recipe = Arc::new(PostgresDeploymentRepository::new(planned.app.clone()));
        recipe
            .admit_install(
                &actor,
                InstallDeployment {
                    command,
                    intent: intent.clone(),
                },
            )
            .await
            .unwrap();
        let configuration = &published.configuration;
        let preparation = PostgresExecutionPreparationRepository::new(
            planned.app.clone(),
            PlatformPolicyObservation::new(
                configuration.platform_policy().clone(),
                configuration.platform_version().to_owned(),
            )
            .unwrap(),
            DeploymentExecutionProfile::RuntimeNamedV1,
        );
        let prepared = preparation
            .prepare_execution(&actor, command, intent.id())
            .await
            .unwrap();
        assert_eq!(prepared.preparation.instances().len(), 1);
        let input = prepared.preparation.instances()[0].clone();
        assert_eq!(input.resource().as_str(), "sqlite");
        assert_eq!(
            input.release_pin().release_id.as_uuid(),
            published.published.release_id
        );
        assert_eq!(
            input.release_pin().release_agent_id.as_uuid(),
            published.published.release_agent_id
        );
        assert_eq!(input.volumes().len(), published.variant.slots().len());
        Self {
            published,
            actor,
            app: planned.app,
            worker: planned.worker,
            intent,
            command,
            recipe,
            input,
            historical_migrations: planned.historical_migrations,
            stopped: planned.stopped,
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

pub struct InvocationBirth {
    pub bootstrap: Arc<Bootstrap>,
    pub physical: OwnedBirths,
    pub runs: Arc<PgRunRepository>,
    pub start: StartRun,
    pub scope: RunCleanupHostId,
    pub vm: VmId,
}
