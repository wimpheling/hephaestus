use identity_domain::RequestId;
use recipe_application::{DeploymentExecutionPreparationRepository, InstallProgress};
use sqlx::PgPool;

use crate::{
    fixtures,
    instance_fixture::{self, Harness},
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn slotless_real_import_commits_named_instance_revision_outboxes_and_actual_ledger_completion(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let result = h
        .importer()
        .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
    assert_eq!(result.install, InstallProgress::Ready);
    assert!(result.active_attempt.is_none());
    let prepared = h.prepared().await;
    let facts:(String,bool,uuid::Uuid,uuid::Uuid,bool,i64,i64,i64,i64,bool,bool,bool)=sqlx::query_as(
      "SELECT instance.volume_mode,instance.run_gate_open,instance.active_revision_id,instance.created_by,revision.runnable,
        (SELECT count(*) FROM recipe_effect_attempts),(SELECT count(*) FROM recipe_execution_transitions WHERE kind='completed'),
        (SELECT count(*) FROM recipe_owned_volume_preparations),(SELECT count(*) FROM agent_instance_volume_mount_grants),
        EXISTS(SELECT 1 FROM agent_instance_events WHERE instance_id=instance.id AND revision_id=revision.id AND event_type='instance.created' AND actor_id=$2),
        EXISTS(SELECT 1 FROM outbox WHERE aggregate_id=instance.id AND subject='hephaestus.agent_instance.created.v1'),
        EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
          WHERE event.aggregate_type='agent_instance' AND event.aggregate_id=instance.id)
       FROM agent_instances instance JOIN agent_instance_revisions revision ON revision.id=instance.active_revision_id
       WHERE instance.id=$1")
      .bind(prepared.instance_id().as_uuid()).bind(h.actor.user_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        (facts.0, facts.1, facts.2, facts.3, facts.4),
        (
            "named".to_owned(),
            false,
            prepared.revision_id().as_uuid(),
            h.actor.user_id.as_uuid(),
            true
        )
    );
    assert_eq!((facts.5, facts.6, facts.7, facts.8), (1, 1, 0, 0));
    assert!(
        facts.9 && facts.10 && facts.11,
        "actual creation and committed product/domain outboxes"
    );
    let actual_parameter_hash: Vec<u8> =
        sqlx::query_scalar("SELECT parameter_hash FROM agent_instance_revisions WHERE id=$1")
            .bind(prepared.revision_id().as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let empty = release_domain::ParameterDocument::resolve(&[], &std::collections::BTreeMap::new())
        .unwrap();
    assert_eq!(
        actual_parameter_hash,
        empty.hash().as_bytes().as_slice(),
        "empty107 parameters hash empty framed stream, not JSON braces"
    );
    assert!(h.active().await.is_none());
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn completed_new_request_replays_original_ids_receipt_without_import_claim_or_event(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let importer = h.importer();
    let first = importer
        .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
    let header = h
        .preparation
        .prepare_execution(&h.actor, h.command, h.intent.id())
        .await
        .unwrap();
    let before = instance_fixture::state(&pool).await;
    let mut fresh = h.actor.clone();
    fresh.request_id = RequestId::new();
    assert_eq!(
        importer
            .import_prepared_instance(&fresh, h.command, h.intent.id(), &fixtures::key("sqlite"))
            .await
            .unwrap(),
        first
    );
    let replay = h
        .preparation
        .prepare_execution(&fresh, h.command, h.intent.id())
        .await
        .unwrap();
    assert_eq!(header.receipt, replay.receipt);
    assert_eq!(header.preparation, replay.preparation);
    assert_eq!(before, instance_fixture::state(&pool).await);
    let authorized:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM authorization_audit_events WHERE request_id=$1 AND actor_id=$2 AND permission='can_use' AND decision='allow')")
        .bind(fresh.request_id.as_uuid()).bind(fresh.user_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(authorized, "completed replay freshly checks SourceCanUse");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn real_import_commit_gap_replays_same_release_key_and_original_active_claim(pool: PgPool) {
    let h = Harness::new(&pool, false).await;
    let prepared = h.prepared().await;
    let claim = h.claim().await;
    let id = h
        .release
        .import_agent_with_volumes(
            &h.actor,
            hephaestus_app::recipe_instance_import::to_import_command(&prepared),
        )
        .await
        .unwrap();
    assert_eq!(id.as_uuid(), prepared.instance_id().as_uuid());
    let actual_claim = h.active().await.unwrap();
    assert_eq!(actual_claim, claim);
    let mut fresh = h.actor.clone();
    fresh.request_id = RequestId::new();
    let result = h
        .importer()
        .import_prepared_instance(&fresh, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
    assert_eq!(result.install, InstallProgress::Ready);
    let counts:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM recipe_effect_attempts),
      (SELECT count(*) FROM release_command_inbox WHERE command_key=$1),(SELECT count(*) FROM agent_instance_events WHERE instance_id=$2 AND event_type='instance.created'),
      (SELECT count(*) FROM recipe_effect_verifications WHERE attempt_id=$3)")
      .bind(prepared.command_key().as_bytes().as_slice()).bind(prepared.instance_id().as_uuid()).bind(claim.provenance.attempt_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1, 1));
    assert_ne!(
        prepared.operation_id(),
        claim.provenance.attempt_id.as_uuid()
    );
    let provenance: (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT request_id,actor_id FROM release_command_inbox WHERE command_key=$1",
    )
    .bind(prepared.command_key().as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        provenance,
        (h.actor.request_id.as_uuid(), h.actor.user_id.as_uuid())
    );
}
