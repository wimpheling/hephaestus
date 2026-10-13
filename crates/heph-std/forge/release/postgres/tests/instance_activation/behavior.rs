use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::RequestId;
use release_postgres::ReleaseService;
use release_service::{InstanceExecutionError, InstanceExecutionService};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
use volume_domain::{VolumeMountGrantId, VolumeMountRevocationId};
use volume_trait::VolumeMountGrantRepository;

use super::fixture::{Harness, another_command, changed_revision, configuration};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn slotless_birth_activates_once_without_dispatch_or_run(pool: PgPool) {
    let h = Harness::new(&pool).await;
    let first = h
        .service()
        .activate_instance(&h.actor, h.command)
        .await
        .unwrap();
    assert_eq!(first.activated_version(), 2);
    let exact:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_instance_execution_activations a JOIN agent_instances i ON i.id=a.instance_id JOIN release_command_inbox c ON c.command_key=a.command_key JOIN application_events e ON e.id=a.event_id JOIN product_event_outbox p ON p.event_id=e.id JOIN agent_instance_events ie ON ie.id=a.instance_event_id JOIN outbox o ON o.id=a.outbox_id WHERE i.id=$1 AND i.version=2 AND i.run_gate_open AND a.actor_id=$2 AND c.request_id=a.request_id AND e.request_id=a.request_id AND ie.event_type='instance.activated' AND o.event_type='agent_instance.activated.v1' AND (SELECT count(*) FROM agent_instance_execution_activation_audits WHERE activation_id=a.id)=2)")
        .bind(h.command.instance_id().as_uuid()).bind(h.actor.user_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(
        exact,
        "fresh protected command, allow audits and both committed outboxes"
    );
    let unsupported:bool=sqlx::query_scalar("SELECT NOT named_volume_dispatch_supported($1) AND NOT EXISTS(SELECT 1 FROM runs WHERE instance_id=$1)").bind(h.command.instance_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(
        unsupported,
        "activation is not Invocation or legacy named dispatch"
    );
    let error = sqlx::query("SELECT assert_instance_accepts_work($1)")
        .bind(h.command.instance_id().as_uuid())
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires private114, actual mkfs.ext4 and readonly e2fsck"]
async fn bound_birth_requires_actual_owned_readiness_and_current_grant(pool: PgPool) {
    let h = Harness::bound(&pool).await;
    let physical = h.physical.as_ref().unwrap();
    let receipt = h
        .service()
        .activate_instance(&h.actor, h.command)
        .await
        .unwrap();
    assert_eq!(receipt.instance_id(), h.command.instance_id());
    physical.assert_unchanged();
    let grant: Uuid = sqlx::query_scalar(
        "SELECT id FROM agent_instance_volume_mount_grants WHERE instance_revision_id=$1",
    )
    .bind(h.command.expected_revision_id().as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    ReleaseService::new(h.app.clone(), Arc::new(PostgresMelangeAuthorizer))
        .revoke_volume_mount(
            &h.actor,
            VolumeMountRevocationId::from_uuid(Uuid::new_v4()).unwrap(),
            VolumeMountGrantId::from_uuid(grant).unwrap(),
        )
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(matches!(
        h.service().activate_instance(&h.actor, h.command).await,
        Err(InstanceExecutionError::Closed)
    ));
    assert_eq!(h.rows().await, before);
    physical.assert_unchanged();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn new_request_replays_original_receipt_and_changed_key_conflicts(pool: PgPool) {
    let h = Harness::new(&pool).await;
    let first = h
        .service()
        .activate_instance(&h.actor, h.command)
        .await
        .unwrap();
    let before = h.rows().await;
    let mut retry = h.actor.clone();
    retry.request_id = RequestId::new();
    assert_eq!(
        h.service()
            .activate_instance(&retry, h.command)
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        h.rows().await,
        before,
        "no UPDATE, command, event or Run on replay"
    );
    let audits:i64=sqlx::query_scalar("SELECT count(*) FROM authorization_audit_events WHERE request_id=$1 AND decision='allow' AND permission IN ('can_manage','can_use')")
        .bind(retry.request_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(audits >= 2);
    assert!(matches!(
        h.service()
            .activate_instance(&retry, another_command(h.command))
            .await,
        Err(InstanceExecutionError::InputConflict)
    ));
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn configured_platform_and_owner_drift_never_rewrites_original_stamp(pool: PgPool) {
    let h = Harness::new(&pool).await;
    let bad = configuration(
        "vm-owner-fixture",
        "/tmp",
        Uuid::new_v4(),
        "platform/changed",
    );
    let before = h.rows().await;
    assert!(matches!(
        h.configured(bad)
            .activate_instance(&h.actor, h.command)
            .await,
        Err(InstanceExecutionError::ConfigurationConflict)
    ));
    assert_eq!(h.rows().await, before);
    h.service()
        .activate_instance(&h.actor, h.command)
        .await
        .unwrap();
    let after = h.rows().await;
    let changed = configuration(
        "another-vm-owner",
        "/tmp",
        Uuid::new_v4(),
        "platform/activation-v1",
    );
    assert!(matches!(
        h.configured(changed)
            .activate_instance(&h.actor, h.command)
            .await,
        Err(InstanceExecutionError::ConfigurationConflict)
    ));
    assert_eq!(h.rows().await, after);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn stale_revision_and_permanent_removal_cannot_activate_or_reopen(pool: PgPool) {
    let h = Harness::new(&pool).await;
    let before = h.rows().await;
    assert!(matches!(
        h.service()
            .activate_instance(&h.actor, changed_revision(h.command))
            .await,
        Err(InstanceExecutionError::StaleInstance)
    ));
    assert_eq!(h.rows().await, before);
    sqlx::query("UPDATE agent_instances SET state='disabled' WHERE id=$1")
        .bind(h.command.instance_id().as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let disabled = h.rows().await;
    assert!(matches!(
        h.service().activate_instance(&h.actor, h.command).await,
        Err(InstanceExecutionError::Closed)
    ));
    assert_eq!(h.rows().await, disabled);
    sqlx::query("UPDATE agent_instances SET state='active' WHERE id=$1")
        .bind(h.command.instance_id().as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    h.service()
        .activate_instance(&h.actor, h.command)
        .await
        .unwrap();
    let snapshot = h.recipe.inspect(&h.actor, h.intent.id()).await.unwrap();
    h.recipe
        .admit_remove(
            &h.actor,
            crate::fixtures::remove(&h.actor, h.intent.id(), snapshot.version),
        )
        .await
        .unwrap();
    let removed = h.rows().await;
    assert!(matches!(
        h.service().activate_instance(&h.actor, h.command).await,
        Err(InstanceExecutionError::Closed)
    ));
    assert_eq!(h.rows().await, removed);
}

pub fn code(error: &sqlx::Error) -> Option<std::borrow::Cow<'_, str>> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn actual_import_commit_gap_stays_closed_until_exact_original106_completion(pool: PgPool) {
    let h = Harness::pending(&pool).await;
    let before = h.rows().await;
    assert!(matches!(
        h.service().activate_instance(&h.actor, h.command).await,
        Err(InstanceExecutionError::Closed)
    ));
    assert_eq!(
        h.rows().await,
        before,
        "real107 birth alone must not resolve active106 Create"
    );
    let claim = h.pending.as_ref().unwrap();
    recipe_postgres::PostgresEffectVerificationRecorder::new(h.worker.clone())
        .record(
            claim,
            claim.provenance.attempt_id,
            recipe_postgres::VerifiedOutcome::Applied,
        )
        .await
        .unwrap();
    h.recipe
        .complete_resource_effect(
            &h.actor,
            claim,
            recipe_application::EffectEvidence {
                identity: claim.identity,
                input_hash: claim.input_hash,
                outcome: recipe_application::EffectOutcome::Applied,
            },
        )
        .await
        .unwrap();
    h.service()
        .activate_instance(&h.actor, h.command)
        .await
        .unwrap();
}
