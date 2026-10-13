//! A genuinely admitted new external-volume consumer, without rerunning schemas.
use crate::fixture::Fixture;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentExecutionPreparationRepository,
    DeploymentExecutionProfile, DeploymentId, DeploymentIntent, DeploymentKey, EffectClaim,
    EffectEvidence, EffectOutcome, ResourceAction, ResourceEffect,
};
use release_service::InstanceExecutionService;
use run_orchestrator::RunRepository;
use sqlx::PgPool;
use std::collections::BTreeMap;
use uuid::Uuid;
use volume_trait::RunVolumeMetadataRepository;

pub async fn acquire_newer(f: &Fixture) -> runtime_types::RunId {
    let count = f.physical.files.len();
    let prior = prior_generations(f).await;
    assert_eq!(prior.len(), count);
    let actor = crate::support::identity(f.birth.fixture.maintainer);
    assert_eq!(actor.user_id, f.birth.actor.user_id);
    assert_ne!(actor.request_id, f.birth.actor.request_id);
    assert_ne!(actor.idempotency_id, f.birth.actor.idempotency_id);
    let input = prepare(f, &actor).await;
    let execution = release_postgres::PostgresInstanceExecutionService::new(
        f.birth.app.clone(),
        std::sync::Arc::new(authz_postgres::PostgresMelangeAuthorizer),
        f.config.clone(),
    );
    execution
        .activate_instance(
            &actor,
            release_service::ActivateInstance::new(
                release_domain::ReleaseCommandKey::derive(
                    "new-holder-activate",
                    &[input.instance_id().as_uuid().as_bytes()],
                ),
                release_service::InstanceActivationId::from_uuid(Uuid::new_v4()).unwrap(),
                input.instance_id(),
                input.revision_id(),
                1,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let invocation = release_service::InvokeInstance::new(
        release_domain::ReleaseCommandKey::derive("new-holder-invoke", &[id.as_bytes()]),
        release_service::InstanceInvocationId::from_uuid(id).unwrap(),
        runtime_types::RunId::new(),
        runtime_types::CommandId::new(),
        input.instance_id(),
        input.revision_id(),
    )
    .unwrap();
    execution
        .with_qualified_invocation_admission()
        .invoke_instance(&actor, invocation)
        .await
        .unwrap();
    let start: run_domain::StartRun = serde_json::from_value(
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT payload FROM command_inbox WHERE command_id=$1",
        )
        .bind(invocation.start_command_id().as_uuid())
        .fetch_one(&f.pool)
        .await
        .unwrap(),
    )
    .unwrap();
    let vm = vm_trait::VmId(start.run_id.to_string());
    f.repository()
        .create_run_with_vm_plan(&start, &f.scope, &vm)
        .await
        .unwrap();
    f.repository()
        .transition(
            start.run_id,
            run_domain::RunState::LeasingVolume,
            None,
            None,
        )
        .await
        .unwrap();
    let selections = f.volumes().load_run_selections(start.run_id).await.unwrap();
    let now = time::OffsetDateTime::now_utc();
    let leases = f
        .volumes()
        .acquire_run(
            &selections,
            f.scope.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert_eq!(leases.len(), count);
    for lease in &leases {
        let lease = lease.lease();
        let generation = prior
            .iter()
            .find(|(id, _)| *id == lease.volume_id.as_uuid())
            .unwrap()
            .1;
        assert_eq!(lease.fencing_token, generation + 1);
    }
    start.run_id
}

async fn prior_generations(f: &Fixture) -> Vec<(Uuid, i64)> {
    sqlx::query_as(
        "SELECT id,lease_generation FROM agent_instance_state_volumes WHERE id=ANY($1) ORDER BY id",
    )
    .bind(
        f.physical
            .files
            .iter()
            .map(|file| file.0)
            .collect::<Vec<_>>(),
    )
    .fetch_all(&f.pool)
    .await
    .unwrap()
}

pub async fn holder(pool: &PgPool, run: runtime_types::RunId) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('run',(SELECT to_jsonb(r) FROM runs r WHERE id=$1),'leases',(SELECT jsonb_agg(to_jsonb(l) ORDER BY id) FROM agent_instance_volume_leases l WHERE run_id=$1),'volumes',(SELECT jsonb_agg(to_jsonb(v) ORDER BY id) FROM agent_instance_state_volumes v WHERE id IN (SELECT volume_id FROM agent_instance_volume_leases WHERE run_id=$1)))")
        .bind(run.as_uuid()).fetch_one(pool).await.unwrap()
}

async fn intent(f: &Fixture) -> DeploymentIntent {
    let count = f.physical.files.len();
    let declaration = crate::graph::declaration(&f.birth.fixture, count, false)
        .replace(
            "source={type=\"created\",capacity_bytes={source=\"input\",name=\"capacity\"}}",
            "source={type=\"external\"}",
        )
        .replace(
            "recipe_id=\"invocation-lease-native\"",
            "recipe_id=\"cleanup-new-holder\"",
        );
    let parsed = recipe_domain::parse_recipe(declaration.as_bytes()).unwrap();
    let mut catalog = crate::fixtures::catalog(&f.birth.fixture);
    let parameters: serde_json::Value =
        sqlx::query_scalar("SELECT parameter_schema FROM release_agents WHERE id=$1")
            .bind(f.birth.fixture.release_agent)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    catalog.parameters = serde_json::from_value(parameters).unwrap();
    catalog.volume_slots =
        release_domain::effective_volume_slots(&crate::graph::slots(count), false).unwrap();
    let ids:Vec<(String,Uuid,i64)>=sqlx::query_as("SELECT slot_key,b.volume_id,v.capacity_bytes FROM agent_instance_revision_volume_bindings b JOIN agent_instance_state_volumes v ON v.id=b.volume_id WHERE b.instance_revision_id=$1 ORDER BY slot_key")
        .bind(f.birth.input.revision_id().as_uuid()).fetch_all(&f.pool).await.unwrap();
    assert_eq!(ids.len(), count);
    let external = ids
        .into_iter()
        .map(|(slot, id, capacity)| {
            (
                crate::fixtures::key(&slot),
                recipe_domain::ExternalVolume {
                    id: runtime_types::VolumeId::from_uuid(id),
                    capacity_bytes: u64::try_from(capacity).unwrap(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let resolved = parsed
        .resolve(&BTreeMap::new(), &external, &[catalog])
        .unwrap();
    assert_ne!(resolved.recipe_id(), f.birth.intent.resolved().recipe_id());
    DeploymentIntent::new(
        DeploymentId::from_uuid(Uuid::new_v4()).unwrap(),
        forge_domain::ProjectId::from_uuid(f.birth.fixture.consuming_project),
        DeploymentKey::parse("new-holder").unwrap(),
        &parsed,
        &resolved,
    )
    .unwrap()
}

async fn prepare(
    f: &Fixture,
    actor: &AuthenticatedIdentity,
) -> recipe_application::PreparedInstanceImport {
    let intent = intent(f).await;
    let install = crate::fixtures::install(actor, intent.clone());
    let command = install.command;
    assert_ne!(command, f.birth.command);
    f.birth.recipe.admit_install(actor, install).await.unwrap();
    let preparation = recipe_postgres::PostgresExecutionPreparationRepository::new(
        f.birth.app.clone(),
        f.birth.input.source().platform().clone(),
        DeploymentExecutionProfile::RuntimeNamedV1,
    );
    let input = preparation
        .prepare_execution(actor, command, intent.id())
        .await
        .unwrap()
        .preparation
        .instances()[0]
        .clone();
    for name in (0..f.physical.files.len())
        .map(|index| format!("data{index:02}"))
        .chain(std::iter::once("sqlite".to_owned()))
    {
        let snapshot = f.birth.recipe.inspect(actor, intent.id()).await.unwrap();
        let key = crate::fixtures::key(&name);
        let claim = f
            .birth
            .recipe
            .claim_resource_effect(
                actor,
                ResourceEffect {
                    command,
                    deployment_id: intent.id(),
                    resource: key.clone(),
                    expected_deployment_version: snapshot.version,
                    expected_resource_version: snapshot.resources[&key].version,
                    action: if name == "sqlite" {
                        ResourceAction::Create
                    } else {
                        ResourceAction::VerifyExternal
                    },
                    provenance: AttemptProvenance::new(
                        actor,
                        DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                    )
                    .unwrap(),
                },
            )
            .await
            .unwrap();
        if name == "sqlite" {
            release_postgres::ReleaseService::new(
                f.birth.app.clone(),
                std::sync::Arc::new(authz_postgres::PostgresMelangeAuthorizer),
            )
            .import_agent_with_volumes(actor, crate::prepared_import::command(&input))
            .await
            .unwrap();
        }
        complete(f, actor, &claim).await;
    }
    input
}

async fn complete(f: &Fixture, actor: &AuthenticatedIdentity, claim: &EffectClaim) {
    assert_eq!(
        claim.provenance,
        AttemptProvenance::new(actor, claim.provenance.attempt_id).unwrap()
    );
    recipe_postgres::PostgresEffectVerificationRecorder::new(f.birth.worker.clone())
        .record(
            claim,
            claim.provenance.attempt_id,
            recipe_postgres::VerifiedOutcome::Applied,
        )
        .await
        .unwrap();
    f.birth
        .recipe
        .complete_resource_effect(
            actor,
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
