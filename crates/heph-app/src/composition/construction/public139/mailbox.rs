//! Genuine current-authorized allocation and committed mailbox command flow.

use super::setup::Fixture;
use mailbox_dispatch::{MAILBOX_DISPATCH_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxDispatchStore};
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEvent, MailboxEventId, ProducerId,
};
use run_domain::StartRun;
use runtime_types::RunId;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;

pub(super) async fn dispatch(fixture: &Fixture) -> StartRun {
    let store = &fixture.app.mailbox_repository;
    let mailbox = store
        .allocate(&fixture.actor, fixture.instance)
        .await
        .expect("actual current-authorized mailbox allocation");
    let body = b"ordinary139 mailbox body";
    let event = event(mailbox, fixture.instance, body);
    let accepted = store
        .accept(
            fixture.repository.project_id.as_uuid(),
            &event,
            body,
            u32::try_from(body.len()).unwrap(),
        )
        .await
        .expect("actual accepted event and opaque payload");
    let wakes = store
        .claim_outbox(&[MAILBOX_WAKE_SUBJECT], 10)
        .await
        .unwrap();
    let wake = wakes
        .into_iter()
        .find(|r| r.command.event_id == accepted.event_id)
        .expect("committed wake");
    store
        .apply_command(&wake.subject, &wake.command)
        .await
        .expect("actual eligibility operation");
    // No broker was invoked; keep outbox publication evidence unchanged.
    let dispatches = store
        .claim_outbox(&[MAILBOX_DISPATCH_SUBJECT], 10)
        .await
        .unwrap();
    let dispatch = dispatches
        .into_iter()
        .find(|r| r.command.event_id == accepted.event_id)
        .expect("committed dispatch");
    store
        .apply_command(&dispatch.subject, &dispatch.command)
        .await
        .unwrap();
    let start = store
        .claim_dispatch(&dispatch.command)
        .await
        .expect("configured Legacy producer")
        .expect("actual accepted attempt and original Run");
    assert!(
        store
            .claim_dispatch(&dispatch.command)
            .await
            .unwrap()
            .is_none()
    );
    let profile: String = sqlx::query_scalar("SELECT execution_profile FROM runs WHERE id=$1")
        .bind(start.run_id.as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(profile, "legacy_scalar");
    start
}

pub(super) async fn assert_settled_cancel(pool: &PgPool, run: RunId) {
    let exact: bool = sqlx::query_scalar(
        "SELECT execution.state='cleaned_up' AND execution.outcome='cancelled'
            AND attempt.state='failed' AND attempt.authorization_snapshot_id IS NOT NULL
            AND delivery.disposition='retryable' AND delivery.next_eligible_at IS NOT NULL
            AND delivery.terminal_at IS NULL
         FROM runs execution JOIN mailbox_delivery_attempts attempt ON attempt.run_id=execution.id
         JOIN mailbox_deliveries delivery ON delivery.event_id=attempt.event_id WHERE execution.id=$1",
    ).bind(run.as_uuid()).fetch_one(pool).await.expect("actual completion observer settlement");
    assert!(exact);
}

fn event(
    mailbox: mailbox_domain::MailboxId,
    instance: runtime_types::AgentInstanceId,
    body: &[u8],
) -> MailboxEvent {
    MailboxEvent {
        id: MailboxEventId::new(),
        mailbox_id: mailbox,
        instance_id: instance,
        producer_id: ProducerId::parse("ordinary139").unwrap(),
        deduplication_key: DeduplicationKey::parse("ordinary139-event").unwrap(),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").unwrap(),
            EnvelopeRoute::parse("/ordinary139").unwrap(),
            std::collections::BTreeMap::new(),
            ContentMetadata::new(
                BodyReference::new(
                    BodyReferenceId::new(),
                    u32::try_from(body.len()).unwrap(),
                    Sha256::digest(body).into(),
                )
                .unwrap(),
                Some("application/octet-stream".into()),
                Some("identity".into()),
            )
            .unwrap(),
            OffsetDateTime::now_utc(),
            None,
        )
        .unwrap(),
    }
}
