use super::*;

/// Replays the post-activation transport path and verifies durable materialization.
///
/// # Panics
/// Panics when a phase fixture operation or durable assertion fails.
// NATS is optional in the fixture; the PostgreSQL dispatch path remains mandatory.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn prepare(context: IsolatedTransportContext) -> IsolatedRecoveryContext {
    let IsolatedTransportContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        first_instance,
        second_instance,
        revised_first,
        update_release_agent,
        update_release_id,
        deferred_attachment,
        deferred_receive,
        deferred_commit,
        update_id,
        update_candidate_revision,
        mailbox_store,
        mailbox_id,
        nats_event_id,
        deferred_trigger_id,
        volume_id,
        ..
    } = context;
    if let Some(nats_event_id) = nats_event_id {
        let nats_url = std::env::var("HEPHAESTUS_NATS_TEST_URL").expect("NATS URL remains set");
        let nats = async_nats::connect(nats_url)
            .await
            .expect("reconnect update-race NATS");
        let jetstream = async_nats::jetstream::new(nats);
        let consumer = mailbox_dispatch::ensure_mailbox_jetstream_topology(&jetstream)
            .await
            .expect("reopen update-race NATS topology");
        let publisher = mailbox_dispatch::MailboxOutboxPublisher::new(
            jetstream,
            Arc::new(mailbox_store.clone()),
        );
        let activation_wake_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM outbox
             WHERE subject = $1 AND aggregate_id = $2 AND id <> $2
             ORDER BY occurred_at DESC, id DESC LIMIT 1",
        )
        .bind(MAILBOX_WAKE_SUBJECT)
        .bind(nats_event_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load NATS activation wake");
        publisher
            .publish_pending(100)
            .await
            .expect("publish post-activation NATS wake");
        let wake_published: bool =
            sqlx::query_scalar("SELECT published_at IS NOT NULL FROM outbox WHERE id = $1")
                .bind(activation_wake_id)
                .fetch_one(&pool)
                .await
                .expect("inspect published NATS wake");
        assert!(wake_published, "activation wake must be accepted by NATS");
        let mut messages = consumer.messages().await.expect("open NATS consumer");
        let wake_delivery = loop {
            let delivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
                .await
                .expect("receive post-activation NATS wake")
                .expect("NATS stream item")
                .expect("valid NATS message");
            let command: MailboxDispatchCommand =
                serde_json::from_slice(&delivery.payload).expect("NATS activation wake command");
            if delivery.message.subject.as_str() == MAILBOX_WAKE_SUBJECT
                && command.event_id == nats_event_id
            {
                break (delivery, command);
            }
            delivery
                .double_ack()
                .await
                .expect("ack unrelated NATS command");
        };
        mailbox_store
            .apply_command(MAILBOX_WAKE_SUBJECT, &wake_delivery.1)
            .await
            .expect("apply post-activation NATS wake");
        wake_delivery
            .0
            .double_ack()
            .await
            .expect("ack post-activation NATS wake");
        let expected_operation_id =
            MailboxOperationIdentity::dispatch(mailbox_id, nats_event_id, 1).id();
        let activation_dispatch_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM outbox
             WHERE subject = $1 AND aggregate_id = $2 AND id <> $3
             ORDER BY occurred_at DESC, id DESC LIMIT 1",
        )
        .bind(MAILBOX_DISPATCH_SUBJECT)
        .bind(nats_event_id.as_uuid())
        .bind(expected_operation_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load NATS activation dispatch");
        publisher
            .publish_pending(100)
            .await
            .expect("publish post-activation NATS dispatch");
        let dispatch_published: bool =
            sqlx::query_scalar("SELECT published_at IS NOT NULL FROM outbox WHERE id = $1")
                .bind(activation_dispatch_id)
                .fetch_one(&pool)
                .await
                .expect("inspect published NATS dispatch");
        assert!(
            dispatch_published,
            "activation dispatch must be accepted by NATS"
        );
        let dispatch_delivery = loop {
            let delivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
                .await
                .expect("receive post-activation NATS dispatch")
                .expect("NATS stream item")
                .expect("valid NATS message");
            let command: MailboxDispatchCommand = serde_json::from_slice(&delivery.payload)
                .expect("NATS activation dispatch command");
            if delivery.message.subject.as_str() == MAILBOX_DISPATCH_SUBJECT
                && command.event_id == nats_event_id
            {
                break (delivery, command);
            }
            delivery
                .double_ack()
                .await
                .expect("ack unrelated NATS command");
        };
        assert_eq!(dispatch_delivery.1.operation_id, expected_operation_id);
        mailbox_store
            .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch_delivery.1)
            .await
            .expect("apply post-activation NATS dispatch");
        let nats_run = mailbox_store
            .claim_dispatch(&dispatch_delivery.1)
            .await
            .expect("claim post-activation NATS dispatch")
            .expect("NATS re-wake claims candidate");
        assert_eq!(nats_run.instance_revision_id, update_candidate_revision);
        assert!(
            mailbox_store
                .claim_dispatch(&dispatch_delivery.1)
                .await
                .expect("duplicate post-activation NATS claim")
                .is_none()
        );
        sqlx::query("UPDATE runs SET state = 'cleaned_up', outcome = 'succeeded' WHERE id = $1")
            .bind(nats_run.run_id.as_uuid())
            .execute(&pool)
            .await
            .expect("finish post-activation NATS proof run");
        dispatch_delivery
            .0
            .double_ack()
            .await
            .expect("ack post-activation NATS dispatch");
    }
    let active_after_update: (Uuid, String, bool) = sqlx::query_as(
        "SELECT active_revision_id, state, run_gate_open
         FROM agent_instances WHERE id = $1",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("active candidate");
    assert_eq!(
        active_after_update,
        (
            update_candidate_revision.as_uuid(),
            String::from("active"),
            true
        )
    );
    let completed_update: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM application_events
             WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
               AND event_type = 'agent_instance.changed'
               AND safe_state = 'active'
         )",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical completed update event");
    assert!(completed_update);
    let identity_preserved: (Uuid, bool) = sqlx::query_as(
        "SELECT instance.state_volume_id,
                EXISTS(
                    SELECT 1 FROM agent_attachments
                    WHERE id = $2 AND instance_id = instance.id
                )
         FROM agent_instances AS instance WHERE instance.id = $1",
    )
    .bind(first_instance.as_uuid())
    .bind(deferred_attachment.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("instance identity after update");
    assert_eq!(identity_preserved, (volume_id, true));
    let volume_ready: String =
        sqlx::query_scalar("SELECT state FROM agent_instance_state_volumes WHERE id = $1")
            .bind(volume_id)
            .fetch_one(&pool)
            .await
            .expect("released update volume");
    assert_eq!(volume_ready, "ready");
    let materialized_deferred: (String, Uuid, Uuid, String) = sqlx::query_as(
        "SELECT deferred.state, request.instance_revision_id,
                request.release_agent_id, request.commit_sha
         FROM deferred_agent_triggers AS deferred
         JOIN run_requests AS request ON request.id = deferred.run_request_id
         WHERE deferred.id = $1",
    )
    .bind(deferred_trigger_id)
    .fetch_one(&pool)
    .await
    .expect("materialized deferred trigger");
    assert_eq!(
        materialized_deferred,
        (
            String::from("materialized"),
            update_candidate_revision.as_uuid(),
            update_release_agent.as_uuid(),
            deferred_commit,
        ),
        "deferred work must bind only the revision active after gate reopen"
    );
    let exact_revision_requests: Vec<(Uuid, Uuid, Vec<u8>)> = sqlx::query_as(
        "SELECT request.instance_revision_id, request.release_id,
                revision.parameter_hash
         FROM run_requests AS request
         JOIN agent_instance_revisions AS revision
           ON revision.id = request.instance_revision_id
         WHERE request.receive_id = $1",
    )
    .bind(deferred_receive)
    .fetch_all(&pool)
    .await
    .expect("exact requests across active revisions");
    assert_eq!(exact_revision_requests.len(), 2);
    let prior_request = exact_revision_requests
        .iter()
        .find(|request| request.0 == revised_first.as_uuid())
        .expect("prior revision request");
    let candidate_request = exact_revision_requests
        .iter()
        .find(|request| request.0 == update_candidate_revision.as_uuid())
        .expect("candidate revision request");
    assert_eq!(prior_request.1, release_id.as_uuid());
    assert_ne!(prior_request.1, candidate_request.1);
    assert_ne!(prior_request.2, candidate_request.2);
    sqlx::query(
        "UPDATE run_requests
         SET dispatch_state = 'dispatched'
         WHERE id = (
             SELECT run_request_id
             FROM deferred_agent_triggers WHERE id = $1
         )",
    )
    .bind(deferred_trigger_id)
    .execute(&pool)
    .await
    .expect("simulate deferred request dispatch");

    IsolatedRecoveryContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        first_instance,
        second_instance,
        revised_first,
        update_release_agent,
        update_release_id,
        update_id,
        update_candidate_revision,
    }
}
