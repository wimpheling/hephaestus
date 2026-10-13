use super::*;

/// Verifies the completed forge result, then drives the durable mailbox journey.
#[allow(
    clippy::cognitive_complexity,
    clippy::needless_borrow,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::uninlined_format_args
)]
pub async fn run_result_and_mailbox_phase(
    pool: &sqlx::PgPool,
    root: &Path,
    repository_id: uuid::Uuid,
    run_id: runtime_types::RunId,
    input_commit: &str,
    user_id: UserId,
    project_id: uuid::Uuid,
    seeded_instance: &SeededInstance,
    running: hephaestus_app::RunningHephaestus,
    brokered_fixture: &mut Option<BrokeredFixture>,
    nats_url: &str,
) {
    let (result_ref, result_commit): (String, String) = sqlx::query_as(
        "SELECT result_ref, result_commit
               FROM run_results WHERE run_id = $1 AND state = 'completed'",
    )
    .bind(run_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("completed result");
    let bare = root
        .join("repositories")
        .join(format!("{}.git", repository_id));
    assert_eq!(
        git_output_bare(&bare, &["rev-parse", &result_ref]).await,
        result_commit
    );
    assert_eq!(
        git_output_bare(&bare, &["rev-parse", &format!("{result_commit}^")]).await,
        input_commit
    );
    assert_eq!(
        git_output_bare(&bare, &["show", &format!("{result_commit}:input.txt")]).await,
        "agent edit"
    );

    let permissions: Vec<String> = sqlx::query_scalar(
        "SELECT permission FROM authorization_audit_events
           WHERE actor_id = $1
           ORDER BY permission",
    )
    .bind(user_id.as_uuid())
    .fetch_all(pool)
    .await
    .expect("authorization audit");
    assert!(permissions.iter().any(|value| value == "can_write"));
    assert!(permissions.iter().any(|value| value == "can_execute"));
    assert!(permissions.iter().any(|value| value == "can_use"));
    let mapped_profile: bool = sqlx::query_scalar(
        "SELECT EXISTS(
              SELECT 1 FROM user_profiles
              WHERE user_id = $1
                AND validated_claims->>'sub' = 'golden-subject'
           )",
    )
    .bind(user_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("OIDC profile mapping");
    assert!(mapped_profile);

    // A durable mailbox journey reuses the production daemon, PostgreSQL
    // outbox, JetStream worker, run gate, fenced state volume, runtime
    // authority and configured VM backend. With `HEPHAESTUS_APP_LIBKRUN_E2E`
    // this is an actual libkrun guest, not a provider double.
    // Keep the fixture behind its gate during daemon startup. Startup/update
    // reconciliation must not acquire this state volume before this test's
    // mailbox command reaches the dispatch-time gate recheck.
    sqlx::query("UPDATE agent_instances SET run_gate_open = true WHERE id = $1")
        .bind(seeded_instance.instance)
        .execute(pool)
        .await
        .expect("open mailbox proof run gate");
    let mailbox_repository = PostgresMailboxRepository::new(pool.clone());
    let mailbox_id = MailboxId::new();
    mailbox_repository
        .ensure_mailbox(
            project_id,
            mailbox_id,
            runtime_types::AgentInstanceId::from_uuid(seeded_instance.instance),
        )
        .await
        .expect("create durable golden mailbox");
    let mailbox_body = b"golden-real-mailbox-body";
    let mailbox_event = MailboxEvent {
        id: mailbox_domain::MailboxEventId::new(),
        mailbox_id,
        instance_id: runtime_types::AgentInstanceId::from_uuid(seeded_instance.instance),
        producer_id: ProducerId::parse("golden-daemon-e2e").expect("producer"),
        deduplication_key: DeduplicationKey::parse("golden-mailbox-1").expect("deduplication"),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/mailbox/golden-proof").expect("route"),
            BTreeMap::new(),
            ContentMetadata::new(
                BodyReference::new(
                    BodyReferenceId::new(),
                    u32::try_from(mailbox_body.len()).expect("mailbox body length"),
                    Sha256::digest(mailbox_body).into(),
                )
                .expect("body reference"),
                Some(String::from("application/octet-stream")),
                Some(String::from("identity")),
            )
            .expect("content metadata"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("mailbox envelope"),
    };
    let accepted = mailbox_repository
        .accept(
            project_id,
            &mailbox_event,
            mailbox_body,
            u32::try_from(mailbox_body.len()).expect("mailbox body length"),
        )
        .await
        .expect("accept mailbox event through PostgreSQL");
    let mailbox_completion = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let row = sqlx::query_as::<
                _,
                (
                    String,
                    uuid::Uuid,
                    String,
                    Option<String>,
                    Option<uuid::Uuid>,
                    Option<uuid::Uuid>,
                    Option<i64>,
                ),
            >(
                "SELECT delivery.disposition, attempt.run_id, run.state, run.outcome,
                        attempt.authorization_snapshot_id, attempt.lease_id,
                        attempt.lease_fencing_token
                 FROM mailbox_deliveries AS delivery
                 JOIN mailbox_delivery_attempts AS attempt ON attempt.event_id = delivery.event_id
                 JOIN runs AS run ON run.id = attempt.run_id
                 WHERE delivery.event_id = $1
                 ORDER BY attempt.attempt_number DESC LIMIT 1",
            )
            .bind(accepted.event_id.as_uuid())
            .fetch_optional(pool)
            .await
            .expect("load composed mailbox delivery");
            if let Some((disposition, run_id, state, outcome, snapshot, lease, fence)) = row {
                if disposition == "delivered" {
                    assert_eq!(state, "cleaned_up");
                    assert_eq!(outcome.as_deref(), Some("succeeded"));
                    assert!(snapshot.is_some(), "runtime authority snapshot is durable");
                    assert!(
                        lease.is_some() && fence.is_some(),
                        "fenced state lease is durable"
                    );
                    break run_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    if mailbox_completion.is_err() {
        let evidence: Vec<MailboxTimeoutEvidence> = sqlx::query_as(
            "SELECT attempt.attempt_number, attempt.run_id, delivery.disposition,
                        attempt.state, run.state, run.outcome, run.failure
                   FROM mailbox_deliveries AS delivery
                   JOIN mailbox_delivery_attempts AS attempt ON attempt.event_id = delivery.event_id
                   JOIN runs AS run ON run.id = attempt.run_id
                  WHERE delivery.event_id = $1
                  ORDER BY attempt.attempt_number",
        )
        .bind(accepted.event_id.as_uuid())
        .fetch_all(pool)
        .await
        .expect("load mailbox timeout evidence");
        eprintln!("golden mailbox timeout: evidence={evidence:?}");
    }
    let mailbox_run_id =
        mailbox_completion.expect("mailbox delivery reaches cleaned-up guest result");
    if let Some(fixture) = brokered_fixture.take() {
        fixture.upstream.assert_substituted_request().await;
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state_access_outcome FROM mailbox_delivery_attempts WHERE run_id = $1"
        )
        .bind(mailbox_run_id)
        .fetch_one(pool)
        .await
        .expect("mailbox state access evidence"),
        "completed_access"
    );

    running.shutdown().await.expect("graceful daemon shutdown");
    let (signals, unpublished_signals): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE message_class = 'internal_signal'),
                  count(*) FILTER (
                      WHERE message_class = 'internal_signal' AND published_at IS NULL
                  )
           FROM outbox",
    )
    .fetch_one(pool)
    .await
    .expect("internal signal outbox census");
    assert_eq!(
        signals, 0,
        "legacy informational signals must not be emitted"
    );
    assert_eq!(
        unpublished_signals, 0,
        "legacy informational signals must never remain pending"
    );
    cleanup_streams(&nats_url).await;
}
