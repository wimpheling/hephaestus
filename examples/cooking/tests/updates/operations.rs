// Reuse the update facade imports across focused lifecycle phases.
#[allow(unused_imports)]
use super::*;
pub(crate) fn destination_path(value: &str) -> bool {
    Path::new(value).is_file()
}

pub(crate) async fn create_update(
    context: &CookingUpdateContext<'_>,
    candidate_release_agent_id: Uuid,
    candidate_rule_ids: BrokeredRuleIds,
) -> UpdateIds {
    let audience = "/hephaestus.instance.v1.AgentInstanceService/CreateUpdate";
    let client = instance_client(context.running, context.rpc_token, audience);
    let response = client
        .create_update(CreateUpdateRequest {
            context: request_context("cooking-update").into(),
            instance_id: opaque(context.instance_id).into(),
            expected_revision_id: opaque(context.current_revision_id).into(),
            candidate_release_agent_id: opaque(candidate_release_agent_id).into(),
            parameters: super::super::cooking_builds::cooking_agent_parameters_for_rules(
                candidate_rule_ids.model,
                candidate_rule_ids.relay,
            ),
            brokered_rule_copies: candidate_rule_ids.copies_from(context.brokered_rule_ids),
            selected_policy: RuntimePolicy {
                vcpus: 1,
                memory_mib: 256,
                network: NetworkPolicy::BrokerOnly.into(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .expect("CreateUpdate request");
    let body = response.into_owned();
    UpdateIds {
        update_id: body
            .update_id
            .into_option()
            .expect("CreateUpdate update ID")
            .value
            .parse()
            .expect("CreateUpdate update ID UUID"),
        candidate_revision_id: body
            .candidate_revision_id
            .into_option()
            .expect("CreateUpdate candidate revision ID")
            .value
            .parse()
            .expect("CreateUpdate candidate revision UUID"),
        model_rule_id: candidate_rule_ids.model,
        relay_rule_id: candidate_rule_ids.relay,
    }
}

pub(crate) async fn assert_active_v1_run(pool: &PgPool, run_id: Uuid, revision_id: Uuid) {
    let row: (String, Uuid) = sqlx::query_as(
        "SELECT state, instance_revision_id FROM runs
          WHERE id = $1 AND run_kind = 'normal'",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("active v1 run barrier");
    assert_eq!(row.1, revision_id, "barrier run must use the v1 revision");
    assert!(
        [
            "queued",
            "leasing_volume",
            "provisioning",
            "starting",
            "running"
        ]
        .contains(&row.0.as_str()),
        "barrier run is already terminal: {}",
        row.0
    );
}

pub(crate) async fn recover_update(
    context: &CookingUpdateContext<'_>,
    update_id: Uuid,
    action: &str,
) {
    let audience = "/hephaestus.instance.v1.AgentInstanceService/RecoverUpdate";
    let action = match action {
        "RECOVERY_ACTION_RETRY" => RecoveryAction::Retry,
        "RECOVERY_ACTION_REJECT" => RecoveryAction::Reject,
        "RECOVERY_ACTION_RESUME" => RecoveryAction::Resume,
        _ => panic!("unsupported update recovery action {action}"),
    };
    instance_client(context.running, context.rpc_token, audience)
        .recover_update(RecoverUpdateRequest {
            context: request_context("cooking-recover-update").into(),
            update_id: opaque(update_id).into(),
            action: action.into(),
            ..Default::default()
        })
        .await
        .expect("RecoverUpdate request");
}

/// Proves fast-hook lifecycle ordering without sampling the gate after the
/// RPC returns. A completed rollback or uncertainty transition may legitimately
/// reopen the gate before the caller can inspect it.
pub(crate) async fn assert_update_event_order(
    pool: &PgPool,
    update_id: Uuid,
    terminal_event: &str,
) {
    let rows: Vec<(String, time::OffsetDateTime)> = sqlx::query_as(
        "SELECT event_type, min(occurred_at)
           FROM outbox
          WHERE aggregate_id = $1
            AND event_type IN (
                'agent_update.requested.v1',
                'agent_update.hook_started.v1',
                'agent_update.rejected.v1',
                'agent_update.uncertain.v1'
            )
          GROUP BY event_type
          ORDER BY min(occurred_at)",
    )
    .bind(update_id)
    .fetch_all(pool)
    .await
    .expect("durable update lifecycle events");
    let timestamps = rows
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let requested = timestamps
        .get("agent_update.requested.v1")
        .copied()
        .expect("durable update requested event");
    let started = timestamps
        .get("agent_update.hook_started.v1")
        .copied()
        .expect("durable update hook started event");
    let terminal = timestamps
        .get(terminal_event)
        .copied()
        .expect("durable update terminal event");
    assert!(requested <= started && started <= terminal);
}

pub(crate) async fn wait_for_state(
    context: &CookingUpdateContext<'_>,
    update_id: Uuid,
    wanted: &str,
) {
    let deadline = Instant::now() + context.timeout;
    loop {
        if load_state(context.pool, update_id).await.update == wanted {
            return;
        }
        if Instant::now() >= deadline {
            let lifecycle = query_timeout_lifecycle(context.pool, update_id).await;
            let hook = match lifecycle.as_ref().and_then(|row| row.5) {
                Some(run_id) => query_timeout_hook(context.pool, run_id).await,
                None => None,
            };
            panic!(
                "update {update_id} did not reach state {wanted}; lifecycle={}; hook={}",
                format_timeout_lifecycle(lifecycle.as_ref()),
                format_timeout_hook(hook.as_ref())
            );
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) type TimeoutLifecycle = (
    String,
    String,
    bool,
    Option<Uuid>,
    Uuid,
    Option<Uuid>,
    Option<Uuid>,
    String,
);

pub(crate) type TimeoutHook = (String, Option<String>, Option<i32>, Option<i32>);
