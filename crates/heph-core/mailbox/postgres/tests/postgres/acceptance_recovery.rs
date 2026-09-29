use mailbox_dispatch::{MAILBOX_DISPATCH_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxDispatchStore};
use mailbox_domain::DeduplicationKey;
use std::sync::Arc;
use uuid::Uuid;

use super::acceptance_setup::AcceptanceState;
use super::support::{attach_run_snapshot, event};

// This helper preserves one ordered recovery and reopened-gate proof so the
// assertions cover its transactional state transitions as one scenario.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn run(state: &AcceptanceState, run_id: uuid::Uuid) {
    let pool = &state.pool;
    let fixture = &state.fixture;
    let store = Arc::clone(&state.store);
    let mailbox_id = state.mailbox_id;
    let first = &state.first;
    let snapshot_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO run_authorization_snapshots
             (id, run_id, instance_id, instance_revision_id,
              authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, 'mailbox-proof/v1', $5)",
    )
    .bind(snapshot_id)
    .bind(run_id)
    .bind(fixture.instance.as_uuid())
    .bind(fixture.revision.as_uuid())
    .bind([9_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("persist exact runtime authorization snapshot");
    sqlx::query(
        "UPDATE mailbox_delivery_attempts SET authorization_snapshot_id = $2
         WHERE run_id = $1",
    )
    .bind(run_id)
    .bind(snapshot_id)
    .execute(pool)
    .await
    .expect("record attempt snapshot evidence");
    sqlx::query("UPDATE runs SET state = 'cleaned_up', outcome = 'failed' WHERE id = $1")
        .bind(run_id)
        .execute(pool)
        .await
        .expect("simulate persisted worker result before recovery");
    assert_eq!(store.recover().await.expect("recover durable attempt"), 0);
    let disposition: String =
        sqlx::query_scalar("SELECT disposition FROM mailbox_deliveries WHERE event_id = $1")
            .bind(first.event_id.as_uuid())
            .fetch_one(pool)
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
    .execute(pool)
    .await
    .expect("reconstruct interrupted authorization settlement");
    sqlx::query(
        "UPDATE runs SET failure = 'run authority operation failed: live capability authority was revoked'
         WHERE id = $1",
    )
    .bind(run_id)
    .execute(pool)
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
    .fetch_one(pool)
    .await
    .expect("load stable authorization denial");
    assert_eq!(disposition, "denied");
    assert_eq!(denial_code.as_deref(), Some("runtime_authorization_denied"));

    // A closed instance run gate defers already-accepted work without binding
    // it to a run. When reopened, concurrent consumers race the same stable
    // dispatch command and PostgreSQL admits one stateful attempt only.
    sqlx::query("UPDATE release_agents SET requires_state = true WHERE id = $1")
        .bind(fixture.release_agent.as_uuid())
        .execute(pool)
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
        .execute(pool)
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
        .execute(pool)
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

    // Finish the preceding gate proof before creating two fresh independent
    // events. This keeps the concurrency assertion focused on those events.
    attach_run_snapshot(
        pool,
        runs[0].run_id.as_uuid(),
        fixture.instance,
        fixture.revision,
        "mailbox-gate-proof/v1",
        8,
    )
    .await;
    sqlx::query("UPDATE runs SET state = 'cleaned_up', outcome = 'succeeded' WHERE id = $1")
        .bind(runs[0].run_id.as_uuid())
        .execute(pool)
        .await
        .expect("finish gate proof run");
    let mut first_concurrent = event(mailbox_id, fixture.instance, b"first-stateful-proof");
    first_concurrent.deduplication_key = DeduplicationKey::parse("first-stateful-proof")
        .expect("first concurrent deduplication key");
    let first_concurrent = store
        .accept(
            fixture.project,
            &first_concurrent,
            b"first-stateful-proof",
            u32::try_from(b"first-stateful-proof".len()).expect("body length"),
        )
        .await
        .expect("accept first independent stateful event");
    let mut second_concurrent = event(mailbox_id, fixture.instance, b"second-stateful-proof");
    second_concurrent.deduplication_key = DeduplicationKey::parse("second-stateful-proof")
        .expect("second concurrent deduplication key");
    let second_concurrent = store
        .accept(
            fixture.project,
            &second_concurrent,
            b"second-stateful-proof",
            u32::try_from(b"second-stateful-proof".len()).expect("body length"),
        )
        .await
        .expect("accept second independent stateful event");
    let first_dispatch = mailbox_dispatch::MailboxDispatchCommand {
        operation_id: mailbox_domain::MailboxOperationIdentity::dispatch(
            mailbox_id,
            first_concurrent.event_id,
            1,
        )
        .id(),
        event_id: first_concurrent.event_id,
    };
    let second_dispatch = mailbox_dispatch::MailboxDispatchCommand {
        operation_id: mailbox_domain::MailboxOperationIdentity::dispatch(
            mailbox_id,
            second_concurrent.event_id,
            1,
        )
        .id(),
        event_id: second_concurrent.event_id,
    };
    for (accepted, command) in [
        (first_concurrent.event_id, first_dispatch.clone()),
        (second_concurrent.event_id, second_dispatch.clone()),
    ] {
        let wake = mailbox_dispatch::MailboxDispatchCommand {
            operation_id: mailbox_domain::MailboxOperationId::from_uuid(accepted.as_uuid()),
            event_id: accepted,
        };
        store
            .apply_command(MAILBOX_WAKE_SUBJECT, &wake)
            .await
            .expect("make independent event eligible");
        store
            .apply_command(MAILBOX_DISPATCH_SUBJECT, &command)
            .await
            .expect("verify independent dispatch command");
    }
    let first_store = store.clone();
    let second_store = store.clone();
    let (left, right) = tokio::join!(
        first_store.claim_dispatch(&first_dispatch),
        second_store.claim_dispatch(&second_dispatch)
    );
    let left_won = matches!(left, Ok(Some(_)));
    let right_won = matches!(right, Ok(Some(_)));
    assert_ne!(left_won, right_won, "one claim must defer durably");
    let winner = if left_won {
        assert!(right.is_err());
        left.expect("left stateful claim admitted")
            .expect("left stateful claim admitted")
    } else {
        assert!(left.is_err());
        right
            .expect("right stateful claim admitted")
            .expect("right stateful claim admitted")
    };
    let (winner_event, deferred_event) = if left_won {
        (first_concurrent.event_id, second_concurrent.event_id)
    } else {
        (second_concurrent.event_id, first_concurrent.event_id)
    };
    attach_run_snapshot(
        pool,
        winner.run_id.as_uuid(),
        fixture.instance,
        fixture.revision,
        "mailbox-winner-proof/v1",
        9,
    )
    .await;
    sqlx::query("UPDATE runs SET state = 'cleaned_up', outcome = 'succeeded' WHERE id = $1")
        .bind(winner.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("finish winning stateful run");
    let deferred_dispatch = if left_won {
        second_dispatch
    } else {
        first_dispatch
    };
    let deferred_attempts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_delivery_attempts WHERE event_id = $1")
            .bind(deferred_event.as_uuid())
            .fetch_one(pool)
            .await
            .expect("count deferred attempts before redelivery");
    assert_eq!(
        deferred_attempts, 0,
        "busy dispatch creates no logical attempt"
    );
    let resumed = store
        .claim_dispatch(&deferred_dispatch)
        .await
        .expect("claim deferred stateful delivery")
        .expect("deferred delivery resumes after winner cleanup");
    assert_ne!(resumed.run_id, winner.run_id);
    let run_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM mailbox_delivery_attempts
          WHERE event_id IN ($1, $2)",
    )
    .bind(winner_event.as_uuid())
    .bind(deferred_event.as_uuid())
    .fetch_one(pool)
    .await
    .expect("count independent durable attempts");
    assert_eq!(
        run_count, 2,
        "each independent event has one logical attempt"
    );
}
