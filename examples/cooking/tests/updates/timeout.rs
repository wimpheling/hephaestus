// Reuse the update facade imports across focused lifecycle phases.
#[allow(unused_imports)]
use super::*;
pub(crate) async fn query_timeout_lifecycle(
    pool: &PgPool,
    update_id: Uuid,
) -> Option<TimeoutLifecycle> {
    // Diagnostics are deliberately best-effort. A database timeout or
    // migration mismatch must not replace the original state timeout.
    timeout(
        Duration::from_secs(2),
        sqlx::query_as::<_, TimeoutLifecycle>(
            "SELECT update_record.state, instance.state,
                    instance.run_gate_open, instance.active_revision_id,
                    update_record.candidate_revision_id, update_record.hook_run_id,
                    candidate.release_agent_id,
                    CASE
                      WHEN COALESCE((candidate_agent.update_hook -> 'arguments') ? '--abnormal-fixture', false)
                        THEN 'abnormal'
                      WHEN COALESCE((candidate_agent.update_hook -> 'arguments') ? '--rollback-fixture', false)
                        THEN 'rollback'
                      WHEN COALESCE((candidate_agent.update_hook -> 'arguments') ? '--migrate', false)
                        THEN 'normal'
                      ELSE 'unknown'
                    END
               FROM agent_updates AS update_record
               JOIN agent_instances AS instance
                 ON instance.id = update_record.instance_id
               LEFT JOIN agent_instance_revisions AS candidate
                 ON candidate.id = update_record.candidate_revision_id
               LEFT JOIN release_agents AS candidate_agent
                 ON candidate_agent.id = candidate.release_agent_id
              WHERE update_record.id = $1",
        )
        .bind(update_id)
        .fetch_optional(pool),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten()
}

pub(crate) async fn query_timeout_hook(pool: &PgPool, run_id: Uuid) -> Option<TimeoutHook> {
    timeout(
        Duration::from_secs(2),
        sqlx::query_as::<_, TimeoutHook>(
            "SELECT state, outcome, exit_code, exit_signal
               FROM runs
              WHERE id = $1",
        )
        .bind(run_id)
        .fetch_optional(pool),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten()
}

pub(crate) fn format_timeout_lifecycle(row: Option<&TimeoutLifecycle>) -> String {
    row.map_or_else(
        || String::from("unavailable"),
        |row| {
            format!(
                "update_state={};instance_state={};gate_open={};active_revision_id={:?};candidate_revision_id={};hook_run_id={:?};release_agent_id={:?};hook_mode={}",
                safe_update_state(&row.0),
                safe_instance_state(&row.1),
                row.2,
                row.3,
                row.4,
                row.5,
                row.6,
                row.7,
            )
        },
    )
}

pub(crate) fn format_timeout_hook(row: Option<&TimeoutHook>) -> String {
    row.map_or_else(
        || String::from("unavailable"),
        |row| {
            format!(
                "state={};outcome={};exit_code={:?};exit_signal={:?}",
                safe_run_state(&row.0),
                safe_run_outcome(row.1.as_deref()),
                row.2,
                row.3
            )
        },
    )
}

pub(crate) fn safe_update_state(value: &str) -> &'static str {
    match value {
        "candidate" => "candidate",
        "draining" => "draining",
        "hook_running" => "hook_running",
        "hook_committed" => "hook_committed",
        "activated" => "activated",
        "rejected" => "rejected",
        "compatibility_unknown" => "compatibility_unknown",
        "activation_recovery" => "activation_recovery",
        _ => "unknown",
    }
}

pub(crate) fn safe_instance_state(value: &str) -> &'static str {
    match value {
        "active" => "active",
        "disabled" => "disabled",
        "update_draining" => "update_draining",
        "updating" => "updating",
        "update_rejected" => "update_rejected",
        "paused_unknown_state" => "paused_unknown_state",
        "paused_activation_recovery" => "paused_activation_recovery",
        "recovering" => "recovering",
        "removed" => "removed",
        _ => "unknown",
    }
}

pub(crate) fn safe_run_state(value: &str) -> &'static str {
    match value {
        "queued" => "queued",
        "leasing_volume" => "leasing_volume",
        "provisioning" => "provisioning",
        "starting" => "starting",
        "running" => "running",
        "succeeded" => "succeeded",
        "failed" => "failed",
        "cancelled" => "cancelled",
        "cleaning_up" => "cleaning_up",
        "cleaned_up" => "cleaned_up",
        _ => "unknown",
    }
}

pub(crate) fn safe_run_outcome(value: Option<&str>) -> &'static str {
    match value {
        Some("succeeded") => "succeeded",
        Some("failed") => "failed",
        Some("cancelled") => "cancelled",
        Some(_) => "unknown",
        None => "none",
    }
}

pub(crate) async fn load_state(pool: &PgPool, update_id: Uuid) -> UpdateState {
    let row: (String, String, bool, Option<Uuid>, Uuid) = sqlx::query_as(
        "SELECT update.state, instance.state, instance.run_gate_open,
                instance.active_revision_id, update.candidate_revision_id
           FROM agent_updates update
           JOIN agent_instances instance ON instance.id = update.instance_id
          WHERE update.id = $1",
    )
    .bind(update_id)
    .fetch_one(pool)
    .await
    .expect("update lifecycle projection");
    UpdateState {
        update: row.0,
        instance: row.1,
        run_gate_open: row.2,
        active_revision_id: row.3,
        candidate_revision_id: row.4,
    }
}

pub(crate) fn instance_client(
    running: &hephaestus_app::RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> AgentInstanceServiceClient<connectrpc::client::HttpClient> {
    let uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("instance RPC URI");
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))
                .expect("instance RPC authorization header"),
        )
        .with_default_timeout(Duration::from_secs(30));
    AgentInstanceServiceClient::new(connectrpc::client::HttpClient::plaintext(), config)
}

pub(crate) fn request_context(operation: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: format!("{operation}-{}", Uuid::new_v4()),
        ..Default::default()
    }
}

pub(crate) fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}
