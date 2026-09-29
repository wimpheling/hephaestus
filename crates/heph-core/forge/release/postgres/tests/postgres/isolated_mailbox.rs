use super::*;

/// Seeds the accepted mailbox command and transport replay state.
///
/// # Panics
/// Panics when a fixture operation or publication assertion fails.
// This phase intentionally keeps its setup SQL and immutable publication checks together.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn prepare(context: IsolatedUpdateContext) -> IsolatedGateContext {
    let IsolatedUpdateContext {
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
        prior_request_id,
        deferred_commit,
        update_id,
        update_candidate_revision,
        ..
    } = context;
    let mailbox_store = PostgresMailboxRepository::new(pool.clone());
    let mailbox_id = MailboxId::new();
    mailbox_store
        .ensure_mailbox(fixture.first_project.as_uuid(), mailbox_id, first_instance)
        .await
        .expect("create update-race mailbox");
    let mailbox_body = b"release-update-gate-race";
    let mailbox_event = gate_race_event(
        mailbox_id,
        first_instance,
        mailbox_body,
        "release-update-race",
    );
    let accepted = mailbox_store
        .accept(
            fixture.first_project.as_uuid(),
            &mailbox_event,
            mailbox_body,
            u32::try_from(mailbox_body.len()).expect("bounded body"),
        )
        .await
        .expect("accept update-race mailbox event");
    let wake = MailboxDispatchCommand {
        operation_id: mailbox_domain::MailboxOperationId::from_uuid(accepted.event_id.as_uuid()),
        event_id: accepted.event_id,
    };
    mailbox_store
        .apply_command(MAILBOX_WAKE_SUBJECT, &wake)
        .await
        .expect("apply initial update-race wake");
    let dispatch = MailboxDispatchCommand {
        operation_id: mailbox_domain::MailboxOperationIdentity::dispatch(
            mailbox_id,
            accepted.event_id,
            1,
        )
        .id(),
        event_id: accepted.event_id,
    };
    mailbox_store
        .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch)
        .await
        .expect("verify initial update-race dispatch");
    assert!(
        mailbox_store
            .claim_dispatch(&dispatch)
            .await
            .expect("closed-gate update-race claim")
            .is_none(),
        "the pre-activation dispatch must not create an attempt"
    );
    let nats_event_id = if let Ok(nats_url) = std::env::var("HEPHAESTUS_NATS_TEST_URL") {
        let nats_body = b"release-update-nats-gate-race";
        let nats_event = gate_race_event(
            mailbox_id,
            first_instance,
            nats_body,
            "release-update-nats-race",
        );
        let accepted = mailbox_store
            .accept(
                fixture.first_project.as_uuid(),
                &nats_event,
                nats_body,
                u32::try_from(nats_body.len()).expect("bounded NATS body"),
            )
            .await
            .expect("accept NATS update-race mailbox event");
        let nats = async_nats::connect(nats_url)
            .await
            .expect("connect update-race NATS");
        let jetstream = async_nats::jetstream::new(nats);
        let consumer = mailbox_dispatch::ensure_mailbox_jetstream_topology(&jetstream)
            .await
            .expect("create update-race NATS topology");
        let publisher = mailbox_dispatch::MailboxOutboxPublisher::new(
            jetstream,
            Arc::new(mailbox_store.clone()),
        );
        publisher
            .publish_pending(100)
            .await
            .expect("publish pre-activation update-race commands");
        let mut messages = consumer
            .messages()
            .await
            .expect("open update-race consumer");
        let wake_delivery = loop {
            let delivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
                .await
                .expect("receive pre-activation NATS wake")
                .expect("NATS stream item")
                .expect("valid NATS message");
            let command: MailboxDispatchCommand =
                serde_json::from_slice(&delivery.payload).expect("NATS wake command");
            if delivery.message.subject.as_str() == MAILBOX_WAKE_SUBJECT
                && command.event_id == accepted.event_id
            {
                break delivery;
            }
            delivery
                .double_ack()
                .await
                .expect("ack unrelated NATS command");
        };
        mailbox_store
            .apply_command(
                MAILBOX_WAKE_SUBJECT,
                &serde_json::from_slice(&wake_delivery.payload).expect("wake command"),
            )
            .await
            .expect("apply pre-activation NATS wake");
        wake_delivery
            .double_ack()
            .await
            .expect("ack pre-activation NATS wake");
        publisher
            .publish_pending(100)
            .await
            .expect("publish pre-activation NATS dispatch");
        let dispatch_delivery = loop {
            let delivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
                .await
                .expect("receive pre-activation NATS dispatch")
                .expect("NATS stream item")
                .expect("valid NATS message");
            let command: MailboxDispatchCommand =
                serde_json::from_slice(&delivery.payload).expect("NATS dispatch command");
            if delivery.message.subject.as_str() == MAILBOX_DISPATCH_SUBJECT
                && command.event_id == accepted.event_id
            {
                break (delivery, command);
            }
            delivery
                .double_ack()
                .await
                .expect("ack unrelated NATS command");
        };
        mailbox_store
            .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch_delivery.1)
            .await
            .expect("apply pre-activation NATS dispatch");
        assert!(
            mailbox_store
                .claim_dispatch(&dispatch_delivery.1)
                .await
                .expect("pre-activation NATS claim")
                .is_none(),
            "pre-activation NATS dispatch must not create an attempt"
        );
        dispatch_delivery
            .0
            .double_ack()
            .await
            .expect("ack consumed pre-activation NATS dispatch");
        drop(messages);
        Some(accepted.event_id)
    } else {
        None
    };

    IsolatedGateContext {
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
        prior_request_id,
        deferred_commit,
        update_id,
        update_candidate_revision,
        mailbox_store,
        mailbox_id,
        accepted,
        dispatch,
        nats_event_id,
    }
}
