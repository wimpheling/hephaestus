//! Complete rows for additional collision, credential and event-counter history.
use serde_json::Value;
use sqlx::PgPool;

pub async fn extra(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('run_git_authority_snapshots',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_git_authority_snapshots t),'runtime_git_credentials',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM runtime_git_credentials t),'git_receives',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM git_receives t),'secret_audit_events',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM secret_audit_events t),'brokered_secret_lease_snapshots',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM brokered_secret_lease_snapshots t),'brokered_secret_audit_events',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM brokered_secret_audit_events t),'mailbox_delivery_attempts',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM mailbox_delivery_attempts t),'agent_updates',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM agent_updates t),'application_event_scopes',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM application_event_scopes t),'application_aggregate_versions',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM application_aggregate_versions t),'users',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM users t),'releases',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM releases t),'agent_instance_revisions',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM agent_instance_revisions t))")
        .fetch_one(pool).await.unwrap()
}
