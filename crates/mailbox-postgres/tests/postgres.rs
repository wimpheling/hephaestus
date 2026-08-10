//! Opt-in real `PostgreSQL` and `JetStream` proof for durable mailbox acceptance.

use authz_postgres::begin_actor_transaction;
use futures_util::StreamExt;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use mailbox_dispatch::{
    MAILBOX_DISPATCH_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxDispatchStore, MailboxOutboxPublisher,
    ensure_mailbox_jetstream_topology,
};
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEvent, MailboxEventId, MailboxId, ProducerId,
};
use mailbox_postgres::PostgresMailboxRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, ReleaseId,
};
use serde_json::json;
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::{collections::BTreeMap, env, sync::Arc, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(clippy::too_many_lines)]
async fn acceptance_deduplication_outbox_jetstream_redelivery_and_recovery_are_durable() {
    let (Ok(database_url), Ok(nats_url)) = (
        env::var("HEPHAESTUS_POSTGRES_TEST_URL"),
        env::var("HEPHAESTUS_NATS_TEST_URL"),
    ) else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply mailbox migrations");
    let fixture = seed_instance(&pool).await;
    let store = Arc::new(PostgresMailboxRepository::new(pool.clone()));
    let mailbox_id = MailboxId::new();
    store
        .ensure_mailbox(fixture.project, mailbox_id, fixture.instance)
        .await
        .expect("create instance-owned mailbox");
    let instance_watch_events_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE scope_kind = 'agent_instance' AND scope_id = $1
           AND aggregate_type = 'agent_instance'
           AND event_type = 'agent_instance.changed'",
    )
    .bind(fixture.instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load initial instance watch cursor");
    let body = b"real-postgres-nats-mailbox";
    let first_event = event(mailbox_id, fixture.instance, body);
    let duplicate_event = MailboxEvent {
        id: MailboxEventId::new(),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/mailbox/proof").expect("route"),
            BTreeMap::default(),
            ContentMetadata::new(
                BodyReference::new(
                    BodyReferenceId::new(),
                    u32::try_from(body.len()).expect("body length"),
                    Sha256::digest(body).into(),
                )
                .expect("body reference"),
                Some("application/octet-stream".to_owned()),
                Some("identity".to_owned()),
            )
            .expect("metadata"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("envelope"),
        ..first_event.clone()
    };
    let decoded_length = u32::try_from(body.len()).expect("body length");
    let (first, duplicate) = tokio::join!(
        store.accept(fixture.project, &first_event, body, decoded_length),
        store.accept(fixture.project, &duplicate_event, body, decoded_length),
    );
    let first = first.expect("first durable acceptance");
    let duplicate = duplicate.expect("concurrent duplicate acceptance");
    assert_eq!(first.event_id, duplicate.event_id);
    assert_ne!(first.duplicate, duplicate.duplicate);
    let (payloads, events, deliveries, wake_outbox): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM mailbox_payloads WHERE mailbox_id = $1),
             (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1),
             (SELECT count(*) FROM mailbox_deliveries WHERE event_id = $2),
             (SELECT count(*) FROM outbox WHERE id = $2 AND subject = $3)",
    )
    .bind(mailbox_id.as_uuid())
    .bind(first.event_id.as_uuid())
    .bind(MAILBOX_WAKE_SUBJECT)
    .fetch_one(&pool)
    .await
    .expect("load atomic acceptance evidence");
    assert_eq!((payloads, events, deliveries, wake_outbox), (1, 1, 1, 1));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_events
             WHERE scope_kind = 'agent_instance' AND scope_id = $1
               AND aggregate_type = 'agent_instance'
               AND event_type = 'agent_instance.changed'",
        )
        .bind(fixture.instance.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("mailbox delivery emits a reauthorizable instance wake"),
        instance_watch_events_before + 1
    );
    let (queue_depth, acceptance_to_dispatch_milliseconds): (i64, i64) = sqlx::query_as(
        "SELECT queue_depth, acceptance_to_dispatch_milliseconds
         FROM mailbox_operation_metrics",
    )
    .fetch_one(&pool)
    .await
    .expect("read aggregate-only mailbox operational metrics");
    assert!(queue_depth >= 1);
    // This aggregate includes durable attempts created by earlier workspace
    // fixtures, so only its non-negative timing invariant is local here.
    assert!(acceptance_to_dispatch_milliseconds >= 0);

    let nats = async_nats::connect(nats_url)
        .await
        .expect("connect real NATS");
    let jetstream = async_nats::jetstream::new(nats);
    let consumer = ensure_mailbox_jetstream_topology(&jetstream)
        .await
        .expect("create durable mailbox consumer");
    let publisher = MailboxOutboxPublisher::new(jetstream.clone(), store.clone());
    // Workspace integration tests share the disposable durable outbox. Drain
    // any earlier mailbox commands, then prove this fixture's exact command.
    assert!(
        publisher
            .publish_pending(1_000)
            .await
            .expect("publish outbox")
            >= 1
    );
    assert_eq!(
        publisher
            .publish_pending(1_000)
            .await
            .expect("replay outbox"),
        0
    );

    let mut messages = consumer.messages().await.expect("open durable consumer");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let first_delivery = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let delivery = tokio::time::timeout(remaining, messages.next())
            .await
            .expect("receive initial JetStream delivery")
            .expect("stream item")
            .expect("valid JetStream message");
        let received: mailbox_dispatch::MailboxDispatchCommand =
            serde_json::from_slice(&delivery.payload).expect("identifier-only command");
        if received.event_id == first.event_id {
            break delivery;
        }
        delivery
            .double_ack()
            .await
            .expect("ack earlier fixture command");
    };
    assert_eq!(
        first_delivery.message.subject.as_str(),
        MAILBOX_WAKE_SUBJECT
    );
    let command: mailbox_dispatch::MailboxDispatchCommand =
        serde_json::from_slice(&first_delivery.payload).expect("identifier-only command");
    assert_eq!(command.event_id, first.event_id);
    assert!(
        !first_delivery
            .payload
            .windows(body.len())
            .any(|window| window == body)
    );
    // NAK models a worker crash after receiving but before acknowledging. The
    // exact same stable command must redeliver, while PostgreSQL remains the
    // authority for its one logical eligibility transition.
    first_delivery
        .ack_with(async_nats::jetstream::AckKind::Nak(None))
        .await
        .expect("request redelivery");
    let redelivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
        .await
        .expect("receive redelivery")
        .expect("stream item")
        .expect("valid redelivery");
    let redelivered: mailbox_dispatch::MailboxDispatchCommand =
        serde_json::from_slice(&redelivery.payload).expect("redelivered command");
    assert_eq!(redelivered, command);
    store
        .apply_command(MAILBOX_WAKE_SUBJECT, &redelivered)
        .await
        .expect("atomically make delivery eligible");
    store
        .apply_command(MAILBOX_WAKE_SUBJECT, &redelivered)
        .await
        .expect("duplicate redelivery is idempotent");
    redelivery
        .double_ack()
        .await
        .expect("ack after PostgreSQL commit");
    // Drop the client-side consumer after the wake-up transition commits.
    // Reconstructing the durable consumer models a dispatcher process restart:
    // the next command is consumed by the reopened durable worker, while
    // PostgreSQL still decides whether it can create a logical attempt.
    drop(messages);
    let consumer = ensure_mailbox_jetstream_topology(&jetstream)
        .await
        .expect("reopen durable mailbox consumer after restart");
    let mut messages = consumer
        .messages()
        .await
        .expect("reopen durable consumer message stream");
    assert_eq!(
        publisher
            .publish_pending(10)
            .await
            .expect("publish dispatch"),
        1
    );
    let dispatch = tokio::time::timeout(Duration::from_secs(5), messages.next())
        .await
        .expect("receive dispatch command")
        .expect("stream item")
        .expect("valid dispatch command");
    assert_eq!(dispatch.message.subject.as_str(), MAILBOX_DISPATCH_SUBJECT);
    let dispatch_command: mailbox_dispatch::MailboxDispatchCommand =
        serde_json::from_slice(&dispatch.payload).expect("dispatch command");
    // A dispatch command is a verified, identifier-only wakeup. Its actual
    // compare-and-swap occurs in `claim_dispatch` immediately afterwards.
    store
        .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch_command)
        .await
        .expect("verify committed dispatch command before claiming it");
    let run = store
        .claim_dispatch(&dispatch_command)
        .await
        .expect("durably claim exactly one run")
        .expect("first dispatch creates run");
    assert!(
        store
            .claim_dispatch(&dispatch_command)
            .await
            .expect("duplicate dispatch claim")
            .is_none()
    );
    dispatch
        .double_ack()
        .await
        .expect("ack dispatch after durable claim");

    let snapshot_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO run_authorization_snapshots
             (id, run_id, instance_id, instance_revision_id,
              authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, 'mailbox-proof/v1', $5)",
    )
    .bind(snapshot_id)
    .bind(run.run_id.as_uuid())
    .bind(fixture.instance.as_uuid())
    .bind(fixture.revision.as_uuid())
    .bind([9_u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect("persist exact runtime authorization snapshot");
    sqlx::query(
        "UPDATE mailbox_delivery_attempts SET authorization_snapshot_id = $2
         WHERE run_id = $1",
    )
    .bind(run.run_id.as_uuid())
    .bind(snapshot_id)
    .execute(&pool)
    .await
    .expect("record attempt snapshot evidence");
    sqlx::query("UPDATE runs SET state = 'cleaned_up', outcome = 'failed' WHERE id = $1")
        .bind(run.run_id.as_uuid())
        .execute(&pool)
        .await
        .expect("simulate persisted worker result before recovery");
    assert_eq!(store.recover().await.expect("recover durable attempt"), 0);
    let disposition: String =
        sqlx::query_scalar("SELECT disposition FROM mailbox_deliveries WHERE event_id = $1")
            .bind(first.event_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("load recovered disposition");
    assert_eq!(disposition, "retryable");

    // A live capability revocation is terminal for this dispatch decision.
    // Recovery must retain that stable denial rather than converting it into
    // another application retry after a worker crash.
    sqlx::query(
        "UPDATE mailbox_deliveries SET disposition = 'leased', next_eligible_at = NULL
         WHERE event_id = $1",
    )
    .bind(first.event_id.as_uuid())
    .execute(&pool)
    .await
    .expect("reconstruct interrupted authorization settlement");
    sqlx::query(
        "UPDATE runs SET failure = 'run authority operation failed: live capability authority was revoked'
         WHERE id = $1",
    )
    .bind(run.run_id.as_uuid())
    .execute(&pool)
    .await
    .expect("persist redacted authorization denial");
    assert_eq!(
        store.recover().await.expect("recover authorization denial"),
        0
    );
    let (disposition, denial_code): (String, Option<String>) = sqlx::query_as(
        "SELECT disposition, denial_code FROM mailbox_deliveries WHERE event_id = $1",
    )
    .bind(first.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load stable authorization denial");
    assert_eq!(disposition, "denied");
    assert_eq!(denial_code.as_deref(), Some("runtime_authorization_denied"));

    // A closed instance run gate defers already-accepted work without binding
    // it to a run. When reopened, concurrent consumers race the same stable
    // dispatch command and PostgreSQL admits one stateful attempt only.
    sqlx::query("UPDATE release_agents SET requires_state = true WHERE id = $1")
        .bind(fixture.release_agent.as_uuid())
        .execute(&pool)
        .await
        .expect("make gate proof stateful");
    let mut gated_event = event(mailbox_id, fixture.instance, b"stateful-gate-proof");
    gated_event.deduplication_key =
        DeduplicationKey::parse("stateful-gate-proof").expect("distinct deduplication key");
    let gated = store
        .accept(
            fixture.project,
            &gated_event,
            b"stateful-gate-proof",
            u32::try_from(b"stateful-gate-proof".len()).expect("body length"),
        )
        .await
        .expect("accept deferred stateful event");
    let wake = mailbox_dispatch::MailboxDispatchCommand {
        operation_id: mailbox_domain::MailboxOperationId::from_uuid(gated.event_id.as_uuid()),
        event_id: gated.event_id,
    };
    store
        .apply_command(MAILBOX_WAKE_SUBJECT, &wake)
        .await
        .expect("make gated event eligible");
    let dispatch = mailbox_dispatch::MailboxDispatchCommand {
        operation_id: mailbox_domain::MailboxOperationIdentity::dispatch(
            mailbox_id,
            gated.event_id,
            1,
        )
        .id(),
        event_id: gated.event_id,
    };
    store
        .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch)
        .await
        .expect("verify deferred dispatch command");
    sqlx::query("UPDATE agent_instances SET run_gate_open = false WHERE id = $1")
        .bind(fixture.instance.as_uuid())
        .execute(&pool)
        .await
        .expect("close run gate");
    assert!(
        store
            .claim_dispatch(&dispatch)
            .await
            .expect("closed-gate claim")
            .is_none()
    );
    sqlx::query("UPDATE agent_instances SET run_gate_open = true WHERE id = $1")
        .bind(fixture.instance.as_uuid())
        .execute(&pool)
        .await
        .expect("reopen run gate");
    let (left, right) = tokio::join!(
        store.claim_dispatch(&dispatch),
        store.claim_dispatch(&dispatch)
    );
    let runs = [
        left.expect("first concurrent claim"),
        right.expect("second concurrent claim"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    assert_eq!(runs.len(), 1, "one stateful run crosses the reopened gate");
    assert!(runs[0].requires_state);
}

/// Proves the mailbox records remain inspectable after their owner is removed,
/// while RLS permits only an authorized project owner to see them.  The test
/// intentionally uses the non-bypass application role rather than the worker
/// role used to arrange the fixture.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn mailbox_rls_isolates_tenants_and_preserves_removed_owner_history() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply mailbox migrations");
    let fixture = seed_instance(&pool).await;
    let store = PostgresMailboxRepository::new(pool.clone());
    let mailbox_id = MailboxId::new();
    store
        .ensure_mailbox(fixture.project, mailbox_id, fixture.instance)
        .await
        .expect("create instance-owned mailbox");
    let body = b"tombstone-safe-mailbox-history";
    let accepted = store
        .accept(
            fixture.project,
            &event(mailbox_id, fixture.instance, body),
            body,
            u32::try_from(body.len()).expect("body length"),
        )
        .await
        .expect("accept event before removal");
    // This test deliberately stops at persistence and inspection.  Mark its
    // wake command settled so the separate transport test owns the shared
    // disposable JetStream consumer regardless of Tokio test ordering.
    sqlx::query("UPDATE outbox SET published_at = now() WHERE id = $1")
        .bind(accepted.event_id.as_uuid())
        .execute(&pool)
        .await
        .expect("isolate RLS fixture from transport proof");

    assert_eq!(
        visible_event_count(&pool, &fixture.owner, mailbox_id).await,
        1
    );
    assert_eq!(
        visible_event_count(&pool, &fixture.outsider, mailbox_id).await,
        0
    );

    // A mailbox tombstone stops new acceptance without deleting immutable event
    // history.  This mirrors a removed instance whose previous delivery/audit
    // evidence must remain available to authorized operators.
    sqlx::query("UPDATE mailboxes SET state = 'removed', removed_at = now() WHERE id = $1")
        .bind(mailbox_id.as_uuid())
        .execute(&pool)
        .await
        .expect("tombstone mailbox");
    assert!(
        store
            .accept(
                fixture.project,
                &event(mailbox_id, fixture.instance, b"must-not-be-accepted"),
                b"must-not-be-accepted",
                20,
            )
            .await
            .is_err()
    );
    assert_eq!(
        visible_event_count(&pool, &fixture.owner, mailbox_id).await,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mailbox_events WHERE id = $1")
            .bind(accepted.event_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("read immutable retained event"),
        1
    );
}

/// Proves expiry never removes bytes from an active delivery, and that the
/// worker cleanup keeps immutable event/hash evidence after a terminal one.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn mailbox_payload_retention_purges_only_expired_terminal_body_bytes() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply mailbox migrations");
    let fixture = seed_instance(&pool).await;
    let store = PostgresMailboxRepository::new(pool.clone());
    let mailbox_id = MailboxId::new();
    store
        .ensure_mailbox(fixture.project, mailbox_id, fixture.instance)
        .await
        .expect("create instance-owned mailbox");
    let body = b"retained-until-terminal";
    let accepted = store
        .accept(
            fixture.project,
            &event(mailbox_id, fixture.instance, body),
            body,
            u32::try_from(body.len()).expect("body length"),
        )
        .await
        .expect("accept event");
    // This proof owns retention rather than JetStream publication; settling
    // the wake isolates the shared durable consumer for the transport test.
    sqlx::query("UPDATE outbox SET published_at = now() WHERE id = $1")
        .bind(accepted.event_id.as_uuid())
        .execute(&pool)
        .await
        .expect("isolate retention fixture from transport proof");

    let mut retention = pool.begin().await.expect("begin retention recalculation");
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *retention)
        .await
        .expect("use worker role for retention recalculation");
    sqlx::query("SET LOCAL hephaestus.mailbox_payload_retention_recalculation = 'on'")
        .execute(&mut *retention)
        .await
        .expect("enable narrow retention recalculation");
    sqlx::query("UPDATE mailbox_payloads SET retained_until = now() - interval '1 second' WHERE id = (SELECT body_id FROM mailbox_events WHERE id = $1)")
        .bind(accepted.event_id.as_uuid())
        .execute(&mut *retention)
        .await
        .expect("expire body for proof");
    retention
        .commit()
        .await
        .expect("commit retention recalculation");
    assert_eq!(
        store
            .cleanup_expired_payloads(10)
            .await
            .expect("cleanup pending payload"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, Vec<u8>>("SELECT encoded_body FROM mailbox_payloads WHERE id = (SELECT body_id FROM mailbox_events WHERE id = $1)")
            .bind(accepted.event_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("pending body remains"),
        body
    );

    sqlx::query("UPDATE mailbox_deliveries SET disposition = 'cancelled', terminal_at = now() WHERE event_id = $1")
        .bind(accepted.event_id.as_uuid())
        .execute(&pool)
        .await
        .expect("terminal delivery");
    assert_eq!(
        store
            .cleanup_expired_payloads(10)
            .await
            .expect("cleanup terminal payload"),
        1
    );
    let (body, purged_at, digest): (Option<Vec<u8>>, Option<OffsetDateTime>, Vec<u8>) =
        sqlx::query_as(
            "SELECT encoded_body, body_purged_at, integrity_hash
             FROM mailbox_payloads WHERE id = (SELECT body_id FROM mailbox_events WHERE id = $1)",
        )
        .bind(accepted.event_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load redacted payload provenance");
    assert!(body.is_none());
    assert!(purged_at.is_some());
    assert_eq!(
        digest,
        Sha256::digest(b"retained-until-terminal").as_slice()
    );
}

async fn visible_event_count(
    pool: &sqlx::PgPool,
    identity: &AuthenticatedIdentity,
    mailbox_id: MailboxId,
) -> i64 {
    let mut transaction = begin_actor_transaction(pool, identity)
        .await
        .expect("set authenticated actor context");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *transaction)
        .await
        .expect("use non-bypass application role");
    let count = sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
        .bind(mailbox_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .expect("RLS mailbox event inspection");
    transaction.commit().await.expect("commit inspection");
    count
}

fn event(mailbox_id: MailboxId, instance_id: AgentInstanceId, body: &[u8]) -> MailboxEvent {
    MailboxEvent {
        id: MailboxEventId::new(),
        mailbox_id,
        instance_id,
        producer_id: ProducerId::parse("real-postgres-nats").expect("producer"),
        deduplication_key: DeduplicationKey::parse("delivery-1").expect("dedup key"),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/mailbox/proof").expect("route"),
            BTreeMap::default(),
            ContentMetadata::new(
                BodyReference::new(
                    BodyReferenceId::new(),
                    u32::try_from(body.len()).expect("body length"),
                    Sha256::digest(body).into(),
                )
                .expect("body reference"),
                Some("application/octet-stream".to_owned()),
                Some("identity".to_owned()),
            )
            .expect("metadata"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("envelope"),
    }
}

struct Fixture {
    project: Uuid,
    instance: AgentInstanceId,
    revision: AgentInstanceRevisionId,
    release_agent: ReleaseAgentId,
    owner: AuthenticatedIdentity,
    outsider: AuthenticatedIdentity,
}

#[allow(clippy::too_many_lines)]
async fn seed_instance(pool: &sqlx::PgPool) -> Fixture {
    let owner = create_user(pool, "Mailbox owner").await;
    let outsider = create_user(pool, "Mailbox outsider").await;
    let organization_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let repository_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let build_request_id = Uuid::new_v4();
    let instance_id = AgentInstanceId::new();
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let revision_id = AgentInstanceRevisionId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("mailbox-{organization_id}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(owner.user_id.as_uuid())
    .execute(pool)
    .await
    .expect("organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("mailbox-{project_id}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository_id)
        .bind(project_id)
        .bind(format!("mailbox-{repository_id}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key) VALUES ($1, $2, 'mailbox')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("family");
    sqlx::query("INSERT INTO build_requests (id, repository_id, source_commit, source_ref, build_definition_hash, state, completed_at) VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())")
        .bind(build_request_id).bind(repository_id).bind("a".repeat(40)).bind([1_u8; 32].as_slice()).execute(pool).await.expect("build");
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state, published_at) VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'published', now())")
        .bind(release_id.as_uuid()).bind(repository_id).bind(format!("mailbox-{release_id}")).bind("a".repeat(40)).bind(build_request_id).bind([1_u8; 32].as_slice()).bind([2_u8; 32].as_slice()).bind([3_u8; 32].as_slice()).execute(pool).await.expect("release");
    sqlx::query("INSERT INTO release_agents (id, release_id, family_id, agent_key, display_name, runtime_contract, runtime_contract_hash, requires_state) VALUES ($1, $2, $3, 'mailbox', 'Mailbox', '{}', $4, false)")
        .bind(release_agent_id.as_uuid()).bind(release_id.as_uuid()).bind(family_id).bind([4_u8; 32].as_slice()).execute(pool).await.expect("release agent");
    sqlx::query("INSERT INTO agent_instances (id, project_id, family_id, name, state) VALUES ($1, $2, $3, $4, 'active')")
        .bind(instance_id.as_uuid()).bind(project_id).bind(family_id).bind(format!("mailbox-{instance_id}")).execute(pool).await.expect("instance");
    sqlx::query("INSERT INTO agent_instance_revisions (id, instance_id, release_agent_id, parameters, parameter_hash, resource_selection, network_restriction, effective_runtime_policy, effective_policy_hash, platform_policy_version, runnable) VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5, 'test/v1', true)")
        .bind(revision_id.as_uuid()).bind(instance_id.as_uuid()).bind(release_agent_id.as_uuid()).bind([5_u8; 32].as_slice()).bind([6_u8; 32].as_slice()).execute(pool).await.expect("revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(pool)
        .await
        .expect("activate revision");
    sqlx::query("INSERT INTO agent_attachments (id, instance_id, project_id, repository_id, ref_selector, trigger_policy) VALUES ($1, $2, $3, $4, 'refs/heads/main', 'manual')")
        .bind(AgentAttachmentId::new().as_uuid()).bind(instance_id.as_uuid()).bind(project_id).bind(repository_id).execute(pool).await.expect("attachment");
    Fixture {
        project: project_id,
        instance: instance_id,
        revision: revision_id,
        release_agent: release_agent_id,
        owner,
        outsider,
    }
}

async fn create_user(pool: &sqlx::PgPool, display_name: &str) -> AuthenticatedIdentity {
    let user_id = UserId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user_id.as_uuid())
        .bind(display_name)
        .execute(pool)
        .await
        .expect("create mailbox test user");
    AuthenticatedIdentity::new(
        user_id,
        "https://issuer.example",
        format!("mailbox-test-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}
