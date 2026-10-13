//! Whole preparation and completion go through the production importer.

use authz_postgres::PostgresMelangeAuthorizer;
use capability_domain::CapabilitySlotKey;
use hephaestus_app::recipe_instance_import::PreparedInstanceImporter;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentExecutionPreparationRepository, DeploymentExecutionProfile,
    DeploymentKey, DeploymentOperation, InstallDeployment, InstallProgress, PlanningRequest,
    PlatformPolicyObservation, PreparedInstanceImport, RecipePlanner,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository, PostgresPlanningCatalog,
};
use release_postgres::ReleaseService;
use release_service::InstanceExecutionConfiguration;
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc};

use crate::fixture::Fixture;

#[path = "parents.rs"]
mod parents;

pub(super) async fn import(
    pool: &PgPool,
    original: &Fixture,
    config: &InstanceExecutionConfiguration,
) -> (AuthenticatedIdentity, PreparedInstanceImport) {
    let platform = PlatformPolicyObservation::new(
        config.platform_policy().clone(),
        config.platform_version().into(),
    )
    .unwrap();
    let (actor, project, release, agent) =
        parents::seed(pool, original, config.platform_policy()).await;
    let declaration = format!(
        "contract_version=1\nrecipe_id=\"invocation-history\"\nrecipe_version=\"1.0.0\"\n[[resources]]\nkind=\"instance\"\nname=\"agent\"\nrelease={{release_id=\"{release}\",release_agent_id=\"{agent}\"}}\nremoval=\"retain\"\n"
    );
    let request = PlanningRequest::new(
        project,
        DeploymentKey::parse("invocation-history").unwrap(),
        declaration.as_bytes(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();
    let intent = RecipePlanner::new(PostgresPlanningCatalog::new(
        original.application.clone(),
        platform.clone(),
    ))
    .plan(&actor, &request)
    .await
    .unwrap()
    .intent()
    .clone();
    let command = CommandIdentity::from_identity(&actor, DeploymentOperation::Install).unwrap();
    let repository = Arc::new(PostgresDeploymentRepository::new(
        original.application.clone(),
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
        original.application.clone(),
        platform,
        DeploymentExecutionProfile::RuntimeNamedV1,
    ));
    let whole = preparation
        .prepare_execution(&actor, command, intent.id())
        .await
        .unwrap();
    assert_eq!(whole.preparation.instances().len(), 1);
    let prepared = whole.preparation.instances()[0].clone();
    assert!(prepared.volumes().is_empty());
    let importer = PreparedInstanceImporter::new(
        preparation,
        repository,
        Arc::new(ReleaseService::new(
            original.application.clone(),
            Arc::new(PostgresMelangeAuthorizer),
        )),
        Arc::new(PostgresEffectVerificationRecorder::new(
            original.worker.clone(),
        )),
    );
    let progress = importer
        .import_prepared_instance(
            &actor,
            command,
            intent.id(),
            &CapabilitySlotKey::parse("agent").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(progress.install, InstallProgress::Ready);
    assert!(progress.active_attempt.is_none());
    let complete: bool =
        sqlx::query_scalar("SELECT qualified_recipe_instance_creation_completed($1)")
            .bind(prepared.instance_id().as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(
        complete,
        "the actual importer committed protected original completion"
    );
    (actor, prepared)
}
