use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentExecutionPreparation, DeploymentExecutionPreparationRepository,
    DeploymentExecutionProfile, DeploymentIntent, ExecutionPreparationAdmission,
    PlatformPolicyObservation,
};
use recipe_postgres::{PostgresDeploymentRepository, PostgresExecutionPreparationRepository};
use sqlx::PgPool;

use crate::{fixtures, schema, source_fixture, support};

#[derive(Clone, Copy)]
pub enum Graph {
    Owned,
    External,
    UnreadyExternal,
    ReferenceOnly,
    TwoInstances,
    BadLastSource,
    ZeroInstances,
}

pub struct Harness {
    pub fixture: support::Fixture,
    pub actor: AuthenticatedIdentity,
    pub intent: DeploymentIntent,
    pub command: CommandIdentity,
    pub app: PgPool,
    pub repository: PostgresDeploymentRepository,
    pub preparation: PostgresExecutionPreparationRepository,
}

pub fn platform(memory: u32, version: &str) -> PlatformPolicyObservation {
    PlatformPolicyObservation::new(source_fixture::policy(memory), version.to_owned()).unwrap()
}

impl Harness {
    pub async fn new(pool: &PgPool, graph: Graph, version: i64) -> Self {
        sqlx::migrate!("../../../../../migrations")
            .run(pool)
            .await
            .unwrap();
        let mut fixture = source_fixture::setup(pool, source_fixture::contract(), true, true).await;
        if matches!(graph, Graph::UnreadyExternal) {
            let volume = uuid::Uuid::new_v4();
            // Intentionally incomplete metadata, not a physical Ready fixture.
            sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,capacity_bytes,state,
              filesystem_uuid,host_id,host_path) SELECT $1,project_id,capacity_bytes,'ready',$2,
              host_id,'/tmp/preparation-not-ready.raw' FROM agent_instance_state_volumes WHERE id=$3")
              .bind(volume).bind(uuid::Uuid::new_v4()).bind(fixture.volume).execute(pool).await.unwrap();
            fixture.volume = volume;
        }
        let app = fixtures::app_pool(pool).await;
        schema::checkpoint(version).run(pool).await.unwrap();
        let actor = support::identity(fixture.maintainer);
        let mut source = declaration(&fixture, graph);
        let mut catalogs = vec![fixtures::catalog(&fixture)];
        if matches!(graph, Graph::BadLastSource) {
            let pin = invalid_source(pool, &fixture).await;
            let old = format!(
                "release = {{ release_id = \"{}\", release_agent_id = \"{}\" }}",
                fixture.release, fixture.release_agent
            );
            let index = source.rfind(&old).unwrap();
            let replacement = format!(
                "release = {{ release_id = \"{}\", release_agent_id = \"{}\" }}",
                pin.release_id, pin.release_agent_id
            );
            source.replace_range(index..index + old.len(), &replacement);
            source = source.replace("name = \"reader\"", "name = \"z_reader\"");
            let mut catalog = fixtures::catalog(&fixture);
            catalog.pin = pin;
            catalogs.push(catalog);
        }
        let external = matches!(
            graph,
            Graph::External | Graph::UnreadyExternal | Graph::ReferenceOnly
        );
        let intent = fixtures::build(
            &fixture,
            &source,
            recipe_application::DeploymentId::from_uuid(uuid::Uuid::new_v4()).unwrap(),
            "sqlite",
            &std::collections::BTreeMap::new(),
            &catalogs,
            external,
        );
        let repository = PostgresDeploymentRepository::new(app.clone());
        let command = fixtures::install(&actor, intent.clone());
        repository
            .admit_install(&actor, command.clone())
            .await
            .unwrap();
        Self {
            fixture,
            actor,
            intent,
            command: command.command,
            repository,
            preparation: PostgresExecutionPreparationRepository::new(
                app.clone(),
                platform(512, "platform/preparation-v1"),
                DeploymentExecutionProfile::RuntimeNamedV1,
            ),
            app,
        }
    }

    pub async fn prepare(&self) -> ExecutionPreparationAdmission {
        self.preparation
            .prepare_execution(&self.actor, self.command, self.intent.id())
            .await
            .unwrap()
    }

    pub fn configured(&self, memory: u32, version: &str) -> PostgresExecutionPreparationRepository {
        PostgresExecutionPreparationRepository::new(
            self.app.clone(),
            platform(memory, version),
            DeploymentExecutionProfile::RuntimeNamedV1,
        )
    }

    pub fn input(&self) -> DeploymentExecutionPreparation {
        let source = recipe_application::SourceObservation::new(
            fixtures::catalog(&self.fixture).pin,
            release_domain::ContentHash::digest(
                &serde_json::to_vec(&source_fixture::contract()).unwrap(),
            ),
            source_fixture::contract()["image_reference"]
                .as_str()
                .unwrap(),
            source_fixture::policy(128),
            platform(512, "platform/preparation-v1"),
        )
        .unwrap();
        DeploymentExecutionPreparation::new(
            &self.intent,
            self.command,
            DeploymentExecutionProfile::RuntimeNamedV1,
            platform(512, "platform/preparation-v1"),
            if self
                .intent
                .resources()
                .values()
                .any(|item| matches!(item.intent(), recipe_domain::ResolvedResource::Instance(_)))
            {
                std::slice::from_ref(&source)
            } else {
                &[]
            },
        )
        .unwrap()
    }

    pub async fn can_use(&self, actor: &AuthenticatedIdentity) -> bool {
        let mut tx = begin_actor_transaction(&self.app, actor).await.unwrap();
        let decision = PostgresMelangeAuthorizer
            .check(
                &mut tx,
                Subject::User(actor.user_id),
                Permission::CanUse,
                ObjectRef::new(ObjectType::ReleaseAgent, self.fixture.release_agent),
            )
            .await
            .unwrap();
        decision == AuthorizationDecision::Allow
    }
}

pub fn declaration(fixture: &support::Fixture, graph: Graph) -> String {
    let source = fixtures::source(
        fixture,
        matches!(
            graph,
            Graph::External | Graph::UnreadyExternal | Graph::ReferenceOnly
        ),
    );
    match graph {
        Graph::ReferenceOnly | Graph::ZeroInstances => source
            .split("[[resources]]\nkind = \"instance\"")
            .next()
            .unwrap()
            .to_owned(),
        Graph::TwoInstances | Graph::BadLastSource => format!(
            "{source}\n[[resources]]\nkind = \"volume\"\nname = \"reader_data\"\nsource = {{ type = \"created\", capacity_bytes = {{ source = \"input\", name = \"capacity\" }} }}\n[[resources]]\nkind = \"instance\"\nname = \"reader\"\nrelease = {{ release_id = \"{}\", release_agent_id = \"{}\" }}\nvolume_bindings = [{{ slot = \"state\", resource = \"reader_data\", guest_path = \"/var/lib/hephaestus\", access_mode = \"read_write\" }}]\nremoval = \"delete\"\n",
            fixture.release, fixture.release_agent
        ),
        Graph::Owned | Graph::External | Graph::UnreadyExternal => source,
    }
}

pub async fn evidence(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
      'headers',(SELECT jsonb_agg(to_jsonb(row) ORDER BY deployment_id) FROM recipe_deployment_execution_preparations row),
      'imports',(SELECT jsonb_agg(to_jsonb(row) ORDER BY deployment_id,ordinal) FROM recipe_prepared_instance_imports row),
      'links',(SELECT jsonb_agg(to_jsonb(row) ORDER BY audit_id) FROM recipe_execution_preparation_audits row),
      'effects',(SELECT count(*) FROM recipe_effect_attempts),
      'mappings',(SELECT count(*) FROM recipe_owned_volume_preparations),
      'grants',(SELECT count(*) FROM agent_instance_volume_mount_grants))")
      .fetch_one(pool).await.unwrap()
}

async fn invalid_source(pool: &PgPool, fixture: &support::Fixture) -> recipe_domain::ReleasePin {
    let release = uuid::Uuid::new_v4();
    let agent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,
      build_definition_hash,configuration,configuration_hash,manifest_hash,state)
      SELECT $1,repository_id,'preparation-bad-last',source_commit,source_ref,build_request_id,
        build_definition_hash,configuration,configuration_hash,manifest_hash,'draft' FROM releases WHERE id=$2")
      .bind(release).bind(fixture.release).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,
      runtime_contract,runtime_contract_hash,requires_state)
      SELECT $1,$2,family_id,'bad-last','Bad last',runtime_contract,$3,requires_state FROM release_agents WHERE id=$4")
      .bind(agent).bind(release).bind([0_u8;32].as_slice()).bind(fixture.release_agent).execute(pool).await.unwrap();
    sqlx::query("UPDATE releases SET state='published',published_at=now() WHERE id=$1")
        .bind(release)
        .execute(pool)
        .await
        .unwrap();
    recipe_domain::ReleasePin {
        release_id: release_domain::ReleaseId::from_uuid(release),
        release_agent_id: release_domain::ReleaseAgentId::from_uuid(agent),
    }
}
