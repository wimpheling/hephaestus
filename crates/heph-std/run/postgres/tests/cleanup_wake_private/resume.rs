use super::{
    observations::{held, preserved_closure, rows},
    wake_fixture::{Wake, service},
};
use recipe_application::{
    CommandIdentity, DeploymentExecutionProfile, DeploymentOperation,
    ExistingInstanceClosureRepository, ExistingInstanceClosureRequest, RemoveDeployment,
    RemoveResumeConfiguration,
};
use recipe_postgres::PostgresDeploymentRepository;
use run_orchestrator::RunRepository;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn fresh_b_resumes_exact_a_closure_after_withdrawal_with_genuine_invocation(pool: PgPool) {
    let h = Wake::historical(crate::fixture::Fixture::new(&pool, 2).await, true, true).await;
    h.prepare().await;
    let manager = crate::support::identity(h.fixture.birth.fixture.member);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.fixture.birth.fixture.consuming_project)
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    h.withdraw().await;
    let snapshot = h
        .fixture
        .birth
        .recipe
        .inspect(&manager, h.fixture.birth.intent.id())
        .await
        .unwrap();
    let remove = RemoveDeployment {
        command: CommandIdentity::from_identity(&manager, DeploymentOperation::Remove).unwrap(),
        deployment_id: h.fixture.birth.intent.id(),
        expected_version: snapshot.version,
    };
    let config = RemoveResumeConfiguration::new(
        h.fixture.scope.provider_namespace().into(),
        h.fixture.scope.host_id().into(),
        h.fixture.birth.input.source().platform().clone(),
        DeploymentExecutionProfile::RuntimeNamedV1,
    )
    .unwrap();
    let repository = PostgresDeploymentRepository::new(h.fixture.birth.app.clone())
        .with_remove_resume(config.clone());
    let unchanged = rows(&pool).await;
    assert!(
        repository
            .admit_remove_resume(&manager, remove)
            .await
            .is_err(),
        "frozen118 requires absent Normal VM admission"
    );
    assert_eq!(rows(&pool).await, unchanged);
    h.upgrade().await;
    mismatched_resume_configuration_is_held(&h, &manager, remove).await;
    let original = preserved_closure(&pool).await;
    let admitted = repository
        .admit_remove_resume(&manager, remove)
        .await
        .unwrap();
    assert_eq!(admitted.continuations().len(), 1);
    let old = &admitted.continuations()[0];
    assert_eq!(old.claim, *h.closure.drain());
    assert_ne!(old.claim.command, remove.command);
    let closure = old.closure.clone().unwrap();
    assert_eq!(closure.removal_id, h.closure.removal_id());
    assert_eq!(closure.command_key, h.closure.command_key());
    assert_eq!(
        closure.expected_version,
        h.request.original().expected_version
    );
    assert_eq!(closure.actor.as_uuid(), h.request.original().creator);
    assert_eq!(closure.request.as_uuid(), h.request.original().request);
    assert_eq!(
        closure.input_hash.as_bytes(),
        &h.request.original().input_hash
    );
    assert_eq!(preserved_closure(&pool).await, original);
    let existing = ExistingInstanceClosureRequest::new(
        remove.command,
        h.fixture.birth.intent.clone(),
        h.closure.target().clone(),
        closure.clone(),
    )
    .unwrap();
    let observed = service(&h.fixture)
        .with_remove_resume(config)
        .unwrap()
        .load_existing_instance_closure(&manager, &existing)
        .await
        .unwrap();
    assert_eq!(observed, closure);
    assert_eq!(preserved_closure(&pool).await, original);
    let prepared = h.read().await;
    assert_eq!(h.prepare().await, prepared);
    changed_wake_config_is_held(&h).await;
    assert_eq!(
        h.request.original().creator,
        h.closure.drain().provenance.actor_id.as_uuid()
    );
    assert_ne!(manager.user_id.as_uuid(), h.request.original().creator);
    h.fixture.physical.assert_unchanged();
}

async fn changed_wake_config_is_held(h: &Wake) {
    // Fresh admission cannot authorize a changed trusted volume namespace/root.
    let wrong = release_service::InstanceExecutionConfiguration::new(
        h.fixture.config.provider_scope().clone(),
        "/different-root".into(),
        h.fixture.config.volume_namespace(),
        h.fixture.config.platform_policy().clone(),
        h.fixture.config.platform_version().into(),
    )
    .unwrap();
    let before = rows(&h.fixture.pool).await;
    held(
        &h.repository()
            .with_qualified_invocation_configuration(wrong)
            .load_cleanup_wake(&h.request, &h.fixture.scope)
            .await
            .unwrap_err(),
    );
    assert_eq!(rows(&h.fixture.pool).await, before);
}

async fn mismatched_resume_configuration_is_held(
    h: &Wake,
    manager: &identity_domain::AuthenticatedIdentity,
    remove: RemoveDeployment,
) {
    let stable = rows(&h.fixture.pool).await;
    let wrong_platform = recipe_application::PlatformPolicyObservation::new(
        crate::source_fixture::policy(1024),
        h.fixture.config.platform_version().into(),
    )
    .unwrap();
    for index in 0..3 {
        let config = RemoveResumeConfiguration::new(
            if index == 0 {
                "foreign".into()
            } else {
                h.fixture.scope.provider_namespace().into()
            },
            if index == 1 {
                "foreign".into()
            } else {
                h.fixture.scope.host_id().into()
            },
            if index == 2 {
                wrong_platform.clone()
            } else {
                h.fixture.birth.input.source().platform().clone()
            },
            DeploymentExecutionProfile::RuntimeNamedV1,
        )
        .unwrap();
        let repository = PostgresDeploymentRepository::new(h.fixture.birth.app.clone())
            .with_remove_resume(config);
        assert!(
            repository
                .admit_remove_resume(manager, remove)
                .await
                .is_err()
        );
        assert_eq!(rows(&h.fixture.pool).await, stable);
    }
}
