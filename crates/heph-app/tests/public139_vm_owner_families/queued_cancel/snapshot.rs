//! Lossless observations of genuine originals and complete neighboring inventories.

use serde_json::Value;
use sqlx::PgPool;

use super::fixture::Fixture;

pub(super) async fn neighbors(pool: &PgPool, f: &Fixture) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'instance',(SELECT to_jsonb(row) FROM agent_instances row WHERE id=$1),
          'other_run',(SELECT to_jsonb(row) FROM runs row WHERE id=$2),
          'other_original',(SELECT to_jsonb(row) FROM agent_instance_execution_invocations row WHERE run_id=$2),
          'other_inbox',(SELECT to_jsonb(row) FROM command_inbox row WHERE command_id=$3),
          'grants',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]'::jsonb) FROM agent_instance_volume_mount_grants row),
          'revocations',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]'::jsonb) FROM agent_instance_volume_mount_revocations row),
          'removals',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]'::jsonb) FROM agent_instance_removal_requests row))",
    ).bind(f.start.instance_id.as_uuid()).bind(f.other.run_id.as_uuid())
        .bind(f.other.command_id.as_uuid()).fetch_one(pool).await.unwrap()
}

pub(super) async fn state(pool: &PgPool, f: &Fixture) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'run',(SELECT to_jsonb(row) FROM runs row WHERE id=$1),
          'original',(SELECT to_jsonb(row) FROM agent_instance_execution_invocations row WHERE run_id=$1),
          'inbox',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY command_id),'[]'::jsonb) FROM command_inbox row WHERE payload->>'run_id'=$1::text),
          'outbox',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]'::jsonb) FROM outbox row WHERE aggregate_id=$1 OR aggregate_id IN (SELECT id FROM control_requests WHERE run_id=$1)),
          'controls',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]'::jsonb) FROM control_requests row WHERE run_id=$1),
          'events',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY sequence),'[]'::jsonb) FROM run_events row WHERE run_id=$1),
          'consumptions',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY run_id),'[]'::jsonb) FROM run_invocation_consumptions row WHERE run_id=$1),
          'targets',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY generation),'[]'::jsonb) FROM run_cleanup_targets row WHERE run_id=$1),
          'fences',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY lease_id),'[]'::jsonb) FROM run_cleanup_target_leases row WHERE run_id=$1),
          'receipts',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY generation),'[]'::jsonb) FROM run_cleanup_receipts row WHERE run_id=$1),
          'completions',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY generation),'[]'::jsonb) FROM run_cleanup_completions row WHERE run_id=$1))",
    ).bind(f.start.run_id.as_uuid()).fetch_one(pool).await.unwrap()
}

pub(super) async fn unconsumed(pool: &PgPool, f: &Fixture) {
    let actual: (i64, bool, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM run_invocation_consumptions WHERE run_id=$1),
         (SELECT processed_at IS NULL FROM command_inbox WHERE command_id=$2),
         (SELECT count(*) FROM run_cleanup_receipts WHERE run_id=$1),
         (SELECT count(*) FROM run_cleanup_completions WHERE run_id=$1)",
    )
    .bind(f.start.run_id.as_uuid())
    .bind(f.start.command_id.as_uuid())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        actual,
        (0, true, 0, 0),
        "no consumption, processed Start or physical receipt"
    );
}
