use hephaestus_app::recipe_engine::{ActivationInspection, RecipeApplicationError};
use recipe_application::{AdmissionDisposition, DeploymentLifecycle, InstallProgress};
use release_service::InstanceExecutionError;
use sqlx::PgPool;

use super::fixture::Fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private114 PostgreSQL, first-Ready guard and native e2fsprogs"]
async fn actual_planner_owned_disk_import_completion_activation_and_immutable_replay(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let request = f.request("engine-install");
    let mut identity = f.h.actor.clone();
    identity.idempotency_id = identity_domain::RequestId::new();
    let first = f.engine.install(&identity, &request).await.unwrap();
    assert_eq!(first.admission.disposition, AdmissionDisposition::Created);
    assert_eq!(first.ledger.lifecycle, DeploymentLifecycle::Installed);
    assert_eq!(
        first.completion.as_ref().unwrap().lifecycle,
        DeploymentLifecycle::Installed
    );
    assert!(
        first
            .ledger
            .resources
            .values()
            .all(|p| p.install == InstallProgress::Ready && p.active_attempt.is_none())
    );
    assert_eq!(first.activations.len(), 1);
    let admitted = first.activations[0].result.as_ref().unwrap();
    assert_eq!(admitted.activated_version(), 2);
    let exact:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_execution_transitions t JOIN recipe_effect_verifications v ON v.id=t.verification_id JOIN recipe_prepared_instance_imports p ON p.deployment_id=t.deployment_id AND p.resource_name=t.resource_name JOIN agent_instance_execution_activations a ON a.instance_id=p.instance_id JOIN agent_instances i ON i.id=a.instance_id WHERE t.deployment_id=$1 AND t.resource_name='sqlite' AND t.after_state->>'install'='ready' AND v.outcome='applied' AND i.run_gate_open AND i.version=2 AND NOT named_volume_dispatch_supported(i.id) AND NOT EXISTS(SELECT 1 FROM runs WHERE instance_id=i.id))")
        .bind(request.id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(
        exact,
        "actual original completion then activation, with no Run or global named support"
    );
    assert_eq!(f.count("formats"), 1);
    let counts = (f.count("formats"), f.count("fscks"));
    let before = f.projection().await;
    identity.request_id = identity_domain::RequestId::new();
    let replay = f.engine.install(&identity, &request).await.unwrap();
    assert_eq!(replay.admission.disposition, AdmissionDisposition::Replay);
    assert!(
        replay.completion.is_none(),
        "no invented historical finish CAS"
    );
    assert_eq!(replay.activations[0].result.as_ref().unwrap(), admitted);
    assert_eq!(f.projection().await, before);
    assert_eq!((f.count("formats"), f.count("fscks")), counts);
    let inspected = f.engine.inspect(&identity, request.id()).await.unwrap();
    assert_eq!(inspected.ledger, replay.ledger);
    assert!(inspected.preparation.is_some());
    assert_eq!(inspected.activation, ActivationInspection::Unsupported);
    assert_input_substitution_is_held(&f, &identity, &request, &before).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires private114 PostgreSQL and native e2fsprogs"]
async fn installed_replay_never_reopens_later_closed_activation_gate(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let first = f.resume().await.unwrap();
    let id = first.activations[0].result.as_ref().unwrap().instance_id();
    sqlx::query("UPDATE agent_instances SET run_gate_open=false WHERE id=$1")
        .bind(id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = f.projection().await;
    let counts = (f.count("formats"), f.count("fscks"));
    let retry = f
        .engine
        .resume(&f.fresh_request(), f.h.command, f.h.intent.id())
        .await
        .unwrap();
    assert_eq!(retry.ledger.lifecycle, DeploymentLifecycle::Installed);
    assert!(retry.completion.is_none());
    assert!(matches!(
        retry.activations[0].result,
        Err(RecipeApplicationError::Activation(
            InstanceExecutionError::Closed
        ))
    ));
    assert_eq!(f.projection().await, before);
    assert_eq!((f.count("formats"), f.count("fscks")), counts);
    assert_eq!(
        f.engine
            .inspect(&f.h.actor, f.h.intent.id())
            .await
            .unwrap()
            .activation,
        ActivationInspection::Unsupported
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires private114 PostgreSQL and native e2fsprogs"]
async fn unsupported_external_recipe_keeps_admitted_intent_without_provider_effects(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    // Real owned creation and original106 completion supply the external catalog
    // fact; reserved legacy seed metadata is not a usable external resource.
    f.create().await.unwrap();
    let original = f.stored_context().await;
    let external = original.claim.purpose.receipt().intent.registration().id();
    let counters = (f.count("formats"), f.count("fscks"));
    let source = crate::fixtures::source(&f.h.fixture, true).replace(
        "recipe_id = \"database\"",
        "recipe_id = \"engine-external\"",
    );
    let request = recipe_application::PlanningRequest::new(
        f.h.intent.project_id(),
        recipe_application::DeploymentKey::parse("external-held").unwrap(),
        source.as_bytes(),
        std::collections::BTreeMap::new(),
        std::collections::BTreeMap::from([(crate::fixtures::key("data"), external)]),
    )
    .unwrap();
    let mut identity = f.h.actor.clone();
    identity.idempotency_id = identity_domain::RequestId::new();
    match f.engine.install(&identity, &request).await {
        Err(RecipeApplicationError::UnsupportedExternal(resource)) => {
            assert_eq!(resource, crate::fixtures::key("data"));
        }
        Err(RecipeApplicationError::Deployment(error)) => {
            panic!("unexpected denial before external boundary: {error}");
        }
        Err(RecipeApplicationError::Volume(_)) => {
            panic!("unexpected volume effect at external boundary");
        }
        Err(RecipeApplicationError::RetainedVolume(_)) => {
            panic!("unexpected retained verification at unconfigured external boundary");
        }
        Err(RecipeApplicationError::Instance(_)) => {
            panic!("unexpected instance effect at external boundary");
        }
        Err(RecipeApplicationError::Activation(_)) => {
            panic!("unexpected activation at external boundary");
        }
        Ok(_) => panic!("external verification unexpectedly executed"),
    }
    let inspected = f.engine.inspect(&identity, request.id()).await.unwrap();
    assert_eq!(inspected.ledger.lifecycle, DeploymentLifecycle::Installing);
    assert!(
        inspected
            .ledger
            .resources
            .values()
            .all(|p| p.install == InstallProgress::Pending && p.active_attempt.is_none())
    );
    assert!(inspected.preparation.is_none());
    assert_eq!((f.count("formats"), f.count("fscks")), counters);
    let effects:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM recipe_effect_attempts WHERE deployment_id=$1)+(SELECT count(*) FROM recipe_owned_volume_preparations WHERE deployment_id=$1)+(SELECT count(*) FROM recipe_prepared_instance_imports WHERE deployment_id=$1)")
        .bind(request.id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(effects, 0);
}

async fn assert_input_substitution_is_held(
    f: &Fixture,
    identity: &identity_domain::AuthenticatedIdentity,
    original: &recipe_application::PlanningRequest,
    before: &serde_json::Value,
) {
    let changed = recipe_application::PlanningRequest::new(
        original.project_id(),
        original.key().clone(),
        f.h.intent.declaration_toml().as_bytes(),
        std::collections::BTreeMap::from([(
            release_domain::ParameterName::parse("capacity").unwrap(),
            release_domain::ParameterValue::Integer(33_554_432),
        )]),
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert!(matches!(
        f.engine.install(identity, &changed).await,
        Err(RecipeApplicationError::Deployment(
            recipe_application::DeploymentError::InputConflict
        ))
    ));
    assert_eq!(&f.projection().await, before);
    assert_eq!(f.count("formats"), 1);
}
