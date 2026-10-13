use identity_domain::RequestId;
use recipe_application::{
    DeploymentError, DeploymentLifecycle, DeploymentRepository, DiagnosticCode, InstallProgress,
    RemovalProgress,
};
use sqlx::PgPool;

use crate::{fixtures, harness::Harness, helpers};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn retirement_finishes_lost_external_reference_without_success_absence_or_external_authority(
    pool: PgPool,
) {
    crate::private_migrations::checkpoint(109)
        .run(&pool)
        .await
        .expect("explicit private109 checkpoint");
    let h = Harness::new(&pool, false, true).await;
    let target = helpers::claim(&h, true).await;
    h.repository
        .mark_resource_ambiguous(&h.identity, &target, DiagnosticCode::ProviderOutcomeUnknown)
        .await
        .expect("uncertainty preserved");
    let manager = helpers::lost_external_manager(&pool, &h).await;
    let retirement = helpers::admit_cleanup(&h, &manager, &target).await;
    let before = helpers::state(&pool, &target).await;
    let port: &dyn DeploymentRepository = &h.repository;
    let retired = port
        .retire_external_observation(&manager, &retirement)
        .await
        .expect("retire read-only observation");
    let mut expected = before.clone();
    expected["active_attempt"] = serde_json::Value::Null;
    expected["version"] = serde_json::json!(before["version"].as_i64().expect("version") + 1);
    assert_eq!(helpers::state(&pool, &target).await, expected);
    assert_eq!(retired.install, InstallProgress::RecoveryRequired);
    assert_eq!(retired.removal, RemovalProgress::Retained);
    assert_eq!(retired.active_attempt, None);
    let ready = port
        .inspect(&manager, h.intent.id())
        .await
        .expect("retained unverified history");
    let terminal = port
        .finish_command(&manager, retirement.command, h.intent.id(), ready.version)
        .await
        .expect("safe reference-only removal");
    assert_eq!(terminal.lifecycle, DeploymentLifecycle::Removed);
    let mut retry = manager.clone();
    retry.request_id = RequestId::new();
    assert_eq!(
        port.retire_external_observation(&retry, &retirement)
            .await
            .expect("unchanged retirement replay after terminal"),
        retired
    );
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM recipe_effect_verifications),
        (SELECT count(*) FROM recipe_execution_transitions WHERE kind='observation_retired')",
    )
    .fetch_one(&pool)
    .await
    .expect("no provider proof");
    assert_eq!(counts, (0, 1));
    let forbidden: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events WHERE actor_id=$1 AND request_id=$2
        AND (permission='can_use' OR object_type='state_volume')",
    )
    .bind(manager.user_id.as_uuid())
    .bind(manager.request_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("cleanup-only audit");
    assert_eq!(forbidden, 0);
    sqlx::query("DELETE FROM project_maintainers WHERE user_id=$1")
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("revoke current manager");
    assert!(matches!(
        port.retire_external_observation(&retry, &retirement).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn selected_external_observation_retires_after_source_and_actor_loss_without_verifying_resource(
    pool: PgPool,
) {
    crate::private_migrations::checkpoint(109)
        .run(&pool)
        .await
        .expect("explicit private109 checkpoint");
    let h = Harness::new(&pool, true, true).await;
    let target = helpers::claim(&h, true).await;
    let manager = helpers::lost_external_manager(&pool, &h).await;
    let retirement = helpers::admit_cleanup(&h, &manager, &target).await;
    let retired = h
        .repository
        .retire_external_observation(&manager, &retirement)
        .await
        .expect("current manager retirement");
    assert_eq!(retired.install, InstallProgress::Creating);
    assert_eq!(retired.removal, RemovalProgress::Retained);
    let snapshot = h
        .repository
        .inspect(&manager, h.intent.id())
        .await
        .expect("pending owned cleanup remains inspectable");
    assert!(matches!(
        h.repository
            .finish_command(
                &manager,
                retirement.command,
                h.intent.id(),
                snapshot.version
            )
            .await,
        Err(DeploymentError::InvalidAction)
    ));
    assert_eq!(
        snapshot.resources[&fixtures::key("sqlite")].removal,
        RemovalProgress::Pending
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn retirement_rejects_forgery_stale_versions_changed_replay_and_unadmitted_command(
    pool: PgPool,
) {
    crate::private_migrations::checkpoint(109)
        .run(&pool)
        .await
        .expect("explicit private109 checkpoint");
    let h = Harness::new(&pool, false, true).await;
    let target = helpers::claim(&h, true).await;
    let cleanup = crate::support::identity(h.fixture.maintainer);
    let command = fixtures::remove(&cleanup, h.intent.id(), 1).command;
    let premature = helpers::retirement(&h, &cleanup, command, &target).await;
    assert!(matches!(
        h.repository
            .retire_external_observation(&cleanup, &premature)
            .await,
        Err(DeploymentError::Unavailable)
    ));
    let retirement = helpers::admit_cleanup(&h, &cleanup, &target).await;
    let mut forged = retirement.clone();
    forged.target.generation += 1;
    assert!(matches!(
        h.repository
            .retire_external_observation(&cleanup, &forged)
            .await,
        Err(DeploymentError::InputConflict)
    ));
    let mut wrong_hash = retirement.clone();
    wrong_hash.target.input_hash = release_domain::ContentHash::digest(b"different intent");
    assert!(matches!(
        h.repository
            .retire_external_observation(&cleanup, &wrong_hash)
            .await,
        Err(DeploymentError::InputConflict)
    ));
    let mut wrong_resource = retirement.clone();
    wrong_resource.target.resource = fixtures::key("another-resource");
    assert!(matches!(
        h.repository
            .retire_external_observation(&cleanup, &wrong_resource)
            .await,
        Err(DeploymentError::InputConflict)
    ));
    let mut stale = retirement.clone();
    stale.expected_resource_version += 1;
    assert!(matches!(
        h.repository
            .retire_external_observation(&cleanup, &stale)
            .await,
        Err(DeploymentError::StaleClaim)
    ));
    h.repository
        .retire_external_observation(&cleanup, &retirement)
        .await
        .expect("retired");
    let mut changed = retirement.clone();
    changed.expected_deployment_version += 1;
    assert!(matches!(
        h.repository
            .retire_external_observation(&cleanup, &changed)
            .await,
        Err(DeploymentError::InputConflict)
    ));
    let prior = helpers::retirement(&h, &cleanup, retirement.command, &target).await;
    assert!(matches!(
        h.repository
            .retire_external_observation(&cleanup, &prior)
            .await,
        Err(DeploymentError::StaleClaim)
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn retirement_never_cancels_owned_creation_or_borrows_an_install_command(pool: PgPool) {
    crate::private_migrations::checkpoint(109)
        .run(&pool)
        .await
        .expect("explicit private109 checkpoint");
    let h = Harness::new(&pool, false, false).await;
    let target = helpers::claim(&h, false).await;
    let cleanup = crate::support::identity(h.fixture.maintainer);
    let retirement = helpers::admit_cleanup(&h, &cleanup, &target).await;
    assert!(matches!(
        h.repository
            .retire_external_observation(&cleanup, &retirement)
            .await,
        Err(DeploymentError::InvalidAction)
    ));
    let mut borrowed = retirement;
    borrowed.command = h.command;
    assert!(matches!(
        h.repository
            .retire_external_observation(&cleanup, &borrowed)
            .await,
        Err(DeploymentError::InputConflict)
    ));
    assert_eq!(
        helpers::state(&pool, &target).await["active_attempt"],
        serde_json::json!(target.provenance.attempt_id.as_uuid())
    );
}
