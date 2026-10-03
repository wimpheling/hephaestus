use recipe_application::{
    AdmissionDisposition, DeploymentError, DeploymentLifecycle, DeploymentRepository,
    RemovalProgress, ResourceAction,
};
use sqlx::PgPool;

use crate::{fixtures, harness::Harness, support};

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn terminal_receipt_is_distinct_immutable_and_replays_exact_original_completion_input(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false, false).await;
    let port: &dyn DeploymentRepository = &h.repository;
    let admission = port
        .admit_install(
            &h.identity,
            fixtures::install(&h.identity, h.intent.clone()),
        )
        .await
        .expect("ongoing admission");
    assert!(matches!(
        port.finish_command(&h.identity, h.command, h.intent.id(), 0)
            .await,
        Err(DeploymentError::InvalidAction)
    ));
    h.apply(&h.request(), h.command, "data", ResourceAction::Create)
        .await;
    let before = port
        .inspect(&h.identity, h.intent.id())
        .await
        .expect("ready");
    let finished = port
        .finish_command(&h.identity, h.command, h.intent.id(), before.version)
        .await
        .expect("terminal receipt");
    assert_eq!(finished.lifecycle, DeploymentLifecycle::Installed);
    assert_ne!(finished.event.event_id, admission.receipt.event.event_id);
    assert!(finished.event.cursor > admission.receipt.event.cursor);
    assert_eq!(finished.input_hash, admission.receipt.input_hash);
    assert_eq!(
        port.finish_command(&h.request(), h.command, h.intent.id(), before.version)
            .await
            .expect("terminal replay"),
        finished
    );
    assert!(matches!(
        port.finish_command(
            &h.request(),
            h.command,
            h.intent.id(),
            finished.deployment_version
        )
        .await,
        Err(DeploymentError::InputConflict)
    ));
    let replay = port
        .admit_install(
            &h.identity,
            fixtures::install(&h.identity, h.intent.clone()),
        )
        .await
        .expect("completed admission replay");
    assert_eq!(replay.disposition, AdmissionDisposition::Replay);
    assert_eq!(replay.receipt, admission.receipt);
    assert_eq!(replay.snapshot.lifecycle, DeploymentLifecycle::Installed);
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM recipe_command_results),
        (SELECT count(*) FROM recipe_execution_transitions WHERE kind='finished')",
    )
    .fetch_one(&pool)
    .await
    .expect("one terminal result");
    assert_eq!(counts, (1, 1));
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn cleanup_runs_reverse_dependencies_and_exact_policy_without_source_access(pool: PgPool) {
    let h = Harness::new(&pool, true, false).await;
    h.apply(&h.request(), h.command, "data", ResourceAction::Create)
        .await;
    h.apply(&h.request(), h.command, "sqlite", ResourceAction::Create)
        .await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.project)
        .bind(h.fixture.maintainer.as_uuid())
        .execute(&pool)
        .await
        .expect("revoke source");
    let observer = h.request();
    let before = h
        .repository
        .inspect(&observer, h.intent.id())
        .await
        .expect("historical inspect");
    assert!(matches!(
        h.repository
            .finish_command(&observer, h.command, h.intent.id(), before.version)
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    let cleanup = support::identity(h.fixture.maintainer);
    let remove = fixtures::remove(&cleanup, h.intent.id(), before.version);
    h.repository
        .admit_remove(&cleanup, remove)
        .await
        .expect("cleanup without source");
    let premature = h
        .effect(&cleanup, remove.command, "data", ResourceAction::Detach)
        .await;
    assert!(matches!(
        h.repository
            .claim_resource_effect(&cleanup, premature)
            .await,
        Err(DeploymentError::InvalidAction)
    ));
    let no_drain = h
        .effect(&cleanup, remove.command, "sqlite", ResourceAction::Detach)
        .await;
    assert!(matches!(
        h.repository.claim_resource_effect(&cleanup, no_drain).await,
        Err(DeploymentError::InvalidAction)
    ));
    for action in [
        ResourceAction::Drain,
        ResourceAction::Detach,
        ResourceAction::Delete,
    ] {
        h.apply(&cleanup, remove.command, "sqlite", action).await;
    }
    h.apply(&cleanup, remove.command, "data", ResourceAction::Detach)
        .await;
    let wrong_policy = h
        .effect(&cleanup, remove.command, "data", ResourceAction::Delete)
        .await;
    assert!(matches!(
        h.repository
            .claim_resource_effect(&cleanup, wrong_policy)
            .await,
        Err(DeploymentError::InvalidAction)
    ));
    h.apply(&cleanup, remove.command, "data", ResourceAction::Retain)
        .await;
    let done = h
        .repository
        .inspect(&cleanup, h.intent.id())
        .await
        .expect("retained owned data");
    assert_eq!(
        done.resources[&fixtures::key("data")].removal,
        RemovalProgress::Retained
    );
    assert_eq!(
        done.resources[&fixtures::key("sqlite")].removal,
        RemovalProgress::Deleted
    );
    let terminal = h
        .repository
        .finish_command(&cleanup, remove.command, h.intent.id(), done.version)
        .await
        .expect("removed");
    assert_eq!(terminal.lifecycle, DeploymentLifecycle::Removed);
    assert_eq!(
        h.repository
            .admit_remove(&cleanup, remove)
            .await
            .expect("removed replay")
            .disposition,
        AdmissionDisposition::Replay
    );
    let source_checks: i64 = sqlx::query_scalar("SELECT count(*) FROM authorization_audit_events WHERE request_id=$1 AND permission='can_use'")
        .bind(cleanup.request_id.as_uuid()).fetch_one(&pool).await.expect("cleanup exact permissions");
    assert_eq!(source_checks, 0);
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn cleanup_finishes_external_reference_without_current_external_authority_or_mutation(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false, true).await;
    h.apply(
        &h.request(),
        h.command,
        "data",
        ResourceAction::VerifyExternal,
    )
    .await;
    let before = h
        .repository
        .inspect(&h.identity, h.intent.id())
        .await
        .expect("external verified");
    sqlx::query("DELETE FROM agent_instance_state_volumes WHERE id=$1")
        .bind(h.fixture.volume)
        .execute(&pool)
        .await
        .expect("external disappears");
    let cleanup = support::identity(h.fixture.maintainer);
    let remove = fixtures::remove(&cleanup, h.intent.id(), before.version);
    let admitted = h
        .repository
        .admit_remove(&cleanup, remove)
        .await
        .expect("no external authority for retention");
    let mutation = h
        .effect(&cleanup, remove.command, "data", ResourceAction::Delete)
        .await;
    assert!(matches!(
        h.repository.claim_resource_effect(&cleanup, mutation).await,
        Err(DeploymentError::InvalidAction)
    ));
    let finished = h
        .repository
        .finish_command(
            &cleanup,
            remove.command,
            h.intent.id(),
            admitted.snapshot.version,
        )
        .await
        .expect("reference retained");
    assert_eq!(finished.lifecycle, DeploymentLifecycle::Removed);
    let mutations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recipe_effect_attempts WHERE action <> 'verify_external'",
    )
    .fetch_one(&pool)
    .await
    .expect("no external mutation attempt");
    assert_eq!(mutations, 0);
}
