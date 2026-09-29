use super::*;

/// Runs the local gate transition and creates the deferred trigger fixture.
///
/// # Panics
/// Panics when a phase fixture operation or durable assertion fails.
// The phase intentionally retains its SQL assertions as one synchronization scenario.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn prepare(context: IsolatedGateContext) -> IsolatedTransportContext {
    let IsolatedGateContext {
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
        ..
    } = context;
    let concurrent_update = service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("concurrent-update", Uuid::new_v4()),
                update_id: AgentUpdateId::new(),
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        concurrent_update,
        Err(release_postgres::ReleaseServiceError::ConcurrentUpdate)
    ));
    let deferred_trigger_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO deferred_agent_triggers
         (id, instance_id, attachment_id, repository_id, target_ref,
          target_commit, source_id)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6)",
    )
    .bind(deferred_trigger_id)
    .bind(first_instance.as_uuid())
    .bind(deferred_attachment.as_uuid())
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(&deferred_commit)
    .bind(deferred_receive)
    .execute(&pool)
    .await
    .expect("defer trigger behind closed gate");
    let draining: (String, String, bool) = sqlx::query_as(
        "SELECT update.state, instance.state, instance.run_gate_open
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("draining update");
    assert_eq!(
        draining,
        (
            String::from("draining"),
            String::from("update_draining"),
            false
        )
    );
    let drain_probe = service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("drain-probe", update_id.as_uuid()),
                update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await;
    assert!(matches!(
        drain_probe,
        Err(release_postgres::ReleaseServiceError::UpdateDrainPending)
    ));
    sqlx::query(
        "UPDATE run_requests SET dispatch_state = 'dispatched'
         WHERE id = $1",
    )
    .bind(prior_request_id)
    .execute(&pool)
    .await
    .expect("drain pre-gate request");
    let volume_id: Uuid =
        sqlx::query_scalar("SELECT state_volume_id FROM agent_instances WHERE id = $1")
            .bind(first_instance.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("instance state volume");
    sqlx::query(
        "UPDATE agent_instance_state_volumes
         SET state = 'ready', host_id = 'test-host',
             host_path = $2, filesystem_uuid = $3
         WHERE id = $1",
    )
    .bind(volume_id)
    .bind(format!("/var/lib/hephaestus-test/{volume_id}"))
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("allocate update volume fixture");
    let hook_run_id = RunId::new();
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("begin-hook", update_id.as_uuid()),
                update_id,
                hook_run_id,
            },
        )
        .await
        .expect("drained update should acquire the fenced volume");
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("begin-hook", update_id.as_uuid()),
                update_id,
                hook_run_id,
            },
        )
        .await
        .expect("duplicate hook admission should resolve idempotently");
    let update_start_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE aggregate_id = $1 AND subject = 'hephaestus.run.start'",
    )
    .bind(hook_run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("one exact update start command");
    assert_eq!(update_start_count, 1);
    let update_start: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM outbox
         WHERE aggregate_id = $1 AND subject = 'hephaestus.run.start'",
    )
    .bind(hook_run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("exact update start command");
    assert_eq!(update_start["kind"], "update");
    assert_eq!(update_start["attachment_id"], serde_json::Value::Null);
    assert_eq!(update_start["run_id"], hook_run_id.to_string());
    assert_eq!(update_start["requires_state"], true);
    sqlx::query(
        "UPDATE runs
         SET state = 'cleaned_up', outcome = 'succeeded', exit_code = 0,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(hook_run_id.as_uuid())
    .execute(&pool)
    .await
    .expect("persist cleaned successful update run");
    let activated = service
        .reconcile_update_run(hook_run_id)
        .await
        .expect("reconcile and activate exact committed candidate");
    assert_eq!(activated, UpdateDecision::Activated);
    let activation_wakes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = $1 AND aggregate_id = $2 AND id <> $2",
    )
    .bind(MAILBOX_WAKE_SUBJECT)
    .bind(accepted.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("count activation mailbox wake");
    assert_eq!(
        activation_wakes, 1,
        "activation must re-wake eligible delivery"
    );
    let activation_wake_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM outbox
         WHERE subject = $1 AND aggregate_id = $2 AND id <> $2
         ORDER BY occurred_at DESC, id DESC LIMIT 1",
    )
    .bind(MAILBOX_WAKE_SUBJECT)
    .bind(accepted.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load activation mailbox wake");
    mailbox_store
        .apply_command(
            MAILBOX_WAKE_SUBJECT,
            &MailboxDispatchCommand {
                operation_id: mailbox_domain::MailboxOperationId::from_uuid(activation_wake_id),
                event_id: accepted.event_id,
            },
        )
        .await
        .expect("apply activation mailbox wake");
    let activation_dispatches: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = $1 AND aggregate_id = $2",
    )
    .bind(MAILBOX_DISPATCH_SUBJECT)
    .bind(accepted.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("count activation dispatch commands");
    assert_eq!(
        activation_dispatches, 2,
        "activation must enqueue one fresh dispatch"
    );
    sqlx::query(
        "INSERT INTO git_refs
         (repository_id, git_ref, commit_sha, updated_by_receive_id)
         VALUES ($1, 'refs/heads/main', $2, $3)",
    )
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(&deferred_commit)
    .bind(deferred_receive)
    .execute(&pool)
    .await
    .expect("seed exact update-race target ref");
    let attempts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_delivery_attempts WHERE event_id = $1")
            .bind(accepted.event_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("count update-race attempts");
    assert_eq!(
        attempts, 0,
        "wake recovery must not create an attempt itself"
    );
    let resumed = mailbox_store
        .claim_dispatch(&dispatch)
        .await
        .expect("claim post-activation update-race dispatch")
        .expect("fresh transport identity claims the candidate once");
    assert_eq!(
        resumed.instance_revision_id, update_candidate_revision,
        "post-activation dispatch must use the active candidate revision"
    );
    sqlx::query(
        "UPDATE runs
         SET state = 'cleaned_up', outcome = 'succeeded', updated_at = now()
         WHERE id = $1",
    )
    .bind(resumed.run_id.as_uuid())
    .execute(&pool)
    .await
    .expect("finish update-race proof run");
    assert!(
        mailbox_store
            .claim_dispatch(&dispatch)
            .await
            .expect("reject duplicate update-race dispatch")
            .is_none(),
        "a stale or duplicate transport command must not create another attempt"
    );

    IsolatedTransportContext {
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
    }
}
