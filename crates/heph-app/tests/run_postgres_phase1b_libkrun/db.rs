use super::model::RunTarget;
use super::*;

pub async fn stage_update_result(
    pool: &PgPool,
    update_id: AgentUpdateId,
    expected: RunTarget,
    candidate: RunTarget,
    hook_run_id: RunId,
) {
    sqlx::query(
        "UPDATE agent_instances
         SET state = 'updating', run_gate_open = false, updated_at = now()
         WHERE id = $1 AND active_revision_id = $2",
    )
    .bind(expected.instance.as_uuid())
    .bind(expected.revision.as_uuid())
    .execute(pool)
    .await
    .expect("close run gate for completed real hook");
    sqlx::query(
        "INSERT INTO agent_updates
         (id, instance_id, expected_current_revision_id,
          candidate_revision_id, state, hook_run_id)
         VALUES ($1, $2, $3, $4, 'hook_running', $5)",
    )
    .bind(update_id.as_uuid())
    .bind(expected.instance.as_uuid())
    .bind(expected.revision.as_uuid())
    .bind(candidate.revision.as_uuid())
    .bind(hook_run_id.as_uuid())
    .execute(pool)
    .await
    .expect("associate real hook with durable update");
}

pub async fn wait_for_state(repository: &PgRunRepository, run_id: RunId, expected: RunState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match repository.get(run_id).await {
            Ok(run) if run.state == expected => return,
            Ok(_) | Err(RepositoryError::NotFound(_)) => {}
            Err(error) => panic!("poll run: {error}"),
        }
        assert!(Instant::now() < deadline, "run never reached {expected:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub async fn sqlite_previous(pool: &PgPool, run_id: RunId) -> u64 {
    let payloads: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM run_events
         WHERE run_id = $1 AND event_type = 'vm.log'
         ORDER BY sequence",
    )
    .bind(run_id.as_uuid())
    .fetch_all(pool)
    .await
    .expect("run log payloads");
    let text = payloads
        .iter()
        .filter_map(|payload| payload.get("bytes").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|byte| u8::try_from(byte).ok())
        .map(char::from)
        .collect::<String>();
    text.lines()
        .find_map(|line| line.strip_prefix("sqlite_previous="))
        .expect("SQLite previous-row marker")
        .parse()
        .expect("valid SQLite previous-row marker")
}

pub async fn log_contains(pool: &PgPool, run_id: RunId, expected: &str) -> bool {
    let payloads: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM run_events
         WHERE run_id = $1 AND event_type = 'vm.log'
         ORDER BY sequence",
    )
    .bind(run_id.as_uuid())
    .fetch_all(pool)
    .await
    .expect("run log payloads");
    payloads
        .iter()
        .filter_map(|payload| payload.get("bytes").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|byte| u8::try_from(byte).ok())
        .map(char::from)
        .collect::<String>()
        .contains(expected)
}

pub async fn cleanup_runtime_records(pool: &PgPool, instance_id: AgentInstanceId) {
    sqlx::query(
        "DELETE FROM outbox WHERE aggregate_id IN (SELECT id FROM runs WHERE instance_id = $1)",
    )
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean outbox");
    sqlx::query(
        "DELETE FROM run_events WHERE run_id IN (SELECT id FROM runs WHERE instance_id = $1)",
    )
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean run events");
    sqlx::query(
        "DELETE FROM command_inbox WHERE command_id IN
         (SELECT command_id FROM runs WHERE instance_id = $1)",
    )
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean command inbox");
    sqlx::query(
        "DELETE FROM outbox
         WHERE aggregate_id IN (
             SELECT id FROM agent_updates WHERE instance_id = $1
         )",
    )
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean update outbox");
    sqlx::query("DELETE FROM agent_updates WHERE instance_id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean updates");
    sqlx::query("UPDATE runs SET lease_id = NULL WHERE instance_id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("detach run lease provenance for fixture cleanup");
    sqlx::query(
        "DELETE FROM agent_instance_volume_leases WHERE volume_id IN
         (SELECT id FROM agent_instance_state_volumes WHERE instance_id = $1)",
    )
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean volume leases");
    sqlx::query("DELETE FROM runs WHERE instance_id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean runs");
    sqlx::query("UPDATE agent_instances SET state_volume_id = NULL WHERE id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("detach instance volume");
    sqlx::query("DELETE FROM agent_instance_state_volumes WHERE instance_id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean volume");
}
