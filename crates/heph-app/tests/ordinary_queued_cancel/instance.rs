use super::fixture::Fixture;
use authz_postgres::PostgresMelangeAuthorizer;
use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use hephaestus_app::recipe_instance_import::PreparedInstanceImporter;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use recipe_application::{
    CommandIdentity, DeploymentExecutionPreparationRepository, DeploymentExecutionProfile,
    DeploymentId, DeploymentKey, DeploymentOperation, InstallDeployment, InstallProgress,
    PlanningRequest, PlatformPolicyObservation, PreparedInstanceImport, RecipePlanner,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository, PostgresPlanningCatalog,
};
use release_postgres::ReleaseService;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;

pub struct Prepared {
    pub actor: AuthenticatedIdentity,
    pub deployment: DeploymentId,
    pub command: CommandIdentity,
    pub instance: PreparedInstanceImport,
    pub repository: Arc<PostgresDeploymentRepository>,
    pub preparation: Arc<PostgresExecutionPreparationRepository>,
    pub recorder: Arc<PostgresEffectVerificationRecorder>,
}

pub fn key(name: &str) -> CapabilitySlotKey {
    CapabilitySlotKey::parse(name).unwrap()
}
pub fn image_reference() -> String {
    format!("registry.example/ordinary-cancel@sha256:{}", "a".repeat(64))
}

pub async fn prepare(fixture: &Fixture) -> Prepared {
    let (actor, project, release, agent) = parents(fixture).await;
    let declaration = format!(
        r#"
contract_version=1
recipe_id="ordinary-queued-cancel"
recipe_version="1.0.0"
[[resources]]
kind="volume"
name="data"
source={{type="created",capacity_bytes={{source="literal",value=16777216}}}}
removal="retain"
[[resources]]
kind="instance"
name="agent"
release={{release_id="{release}",release_agent_id="{agent}"}}
volume_bindings=[{{slot="state",resource="data",guest_path="/var/lib/hephaestus",access_mode="read_write"}}]
removal="retain"
"#
    );
    let platform = PlatformPolicyObservation::new(
        fixture.config.platform_policy().clone(),
        fixture.config.platform_version().into(),
    )
    .unwrap();
    let request = PlanningRequest::new(
        project,
        DeploymentKey::parse("ordinary-queued-cancel").unwrap(),
        declaration.as_bytes(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();
    let intent = RecipePlanner::new(PostgresPlanningCatalog::new(
        fixture.original.application.clone(),
        platform.clone(),
    ))
    .plan(&actor, &request)
    .await
    .unwrap()
    .intent()
    .clone();
    assert_eq!(intent.resources().len(), 2);
    let command = CommandIdentity::from_identity(&actor, DeploymentOperation::Install).unwrap();
    let repository = Arc::new(PostgresDeploymentRepository::new(
        fixture.original.application.clone(),
    ));
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
        fixture.original.application.clone(),
        platform,
        DeploymentExecutionProfile::RuntimeNamedV1,
    ));
    let whole = preparation
        .prepare_execution(&actor, command, intent.id())
        .await
        .unwrap();
    assert_eq!(whole.preparation.instances().len(), 1);
    let instance = whole.preparation.instances()[0].clone();
    assert_eq!(instance.volumes().len(), 1);
    assert_eq!(instance.volumes()[0].slot(), &key("state"));
    assert_eq!(
        instance.volumes()[0].guest_path().as_str(),
        "/var/lib/hephaestus"
    );
    let recorder = Arc::new(PostgresEffectVerificationRecorder::new(
        fixture.original.worker.clone(),
    ));
    Prepared {
        actor,
        deployment: intent.id(),
        command,
        instance,
        repository,
        preparation,
        recorder,
    }
}

pub async fn import(fixture: &Fixture, prepared: &Prepared) {
    let importer = PreparedInstanceImporter::new(
        prepared.preparation.clone(),
        prepared.repository.clone(),
        Arc::new(ReleaseService::new(
            fixture.original.application.clone(),
            Arc::new(PostgresMelangeAuthorizer),
        )),
        prepared.recorder.clone(),
    );
    let progress = importer
        .import_prepared_instance(
            &prepared.actor,
            prepared.command,
            prepared.deployment,
            &key("agent"),
        )
        .await
        .unwrap();
    assert_eq!(progress.install, InstallProgress::Ready);
    assert!(progress.active_attempt.is_none());
    let complete: bool =
        sqlx::query_scalar("SELECT qualified_recipe_instance_creation_completed($1)")
            .bind(prepared.instance.instance_id().as_uuid())
            .fetch_one(&fixture.admin)
            .await
            .unwrap();
    assert!(complete);
}

async fn parents(fixture: &Fixture) -> (AuthenticatedIdentity, ProjectId, Uuid, Uuid) {
    let project: Uuid = sqlx::query_scalar("SELECT project_id FROM repositories WHERE id=$1")
        .bind(fixture.original.repository)
        .fetch_one(&fixture.admin)
        .await
        .unwrap();
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'ordinary cancellation manager')")
        .bind(user)
        .execute(&fixture.admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(user)
        .execute(&fixture.admin)
        .await
        .unwrap();
    let actor = AuthenticatedIdentity::new(
        UserId::from_uuid(user),
        "ordinary-cancel",
        user.to_string(),
        serde_json::json!({}),
        RequestId::new(),
    );
    let release = Uuid::new_v4();
    let agent = Uuid::new_v4();
    // Released parent metadata only: no build publication, Ready, binding or grant proof.
    let contract = serde_json::json!({"executable":"bin/agent","arguments":[],"working_directory":".","image_reference":image_reference(),"requires_state":true,"publication_mode":"proposal","policy_ceiling":fixture.config.platform_policy(),"volume_slots":[]});
    let hash: [u8; 32] = Sha256::digest(serde_json::to_vec(&contract).unwrap()).into();
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state,published_at) SELECT $1,repository_id,'ordinary-cancel-v1',source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'published',now() FROM releases WHERE id=$2").bind(release).bind(fixture.original.start.release_id.as_uuid()).execute(&fixture.admin).await.unwrap();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) SELECT $1,$2,family_id,agent_key,'ordinary stateful cancellation',$3,$4,true FROM release_agents WHERE id=$5").bind(agent).bind(release).bind(contract).bind(hash.as_slice()).bind(fixture.original.start.release_agent_id.as_uuid()).execute(&fixture.admin).await.unwrap();
    (actor, ProjectId::from_uuid(project), release, agent)
}
