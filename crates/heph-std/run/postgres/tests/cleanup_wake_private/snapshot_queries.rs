use serde_json::Value;
use sqlx::{Postgres, postgres::PgArguments, query::QueryScalar};

// Each closed whitelist entry retains the exact complete-row snapshot query.
pub(super) fn table(name: &str) -> QueryScalar<'static, Postgres, Value, PgArguments> {
    match name {
        "runs" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM runs t",
        ),
        "run_events" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM run_events t",
        ),
        "agent_instance_events" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM agent_instance_events t",
        ),
        "run_invocation_consumptions" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM run_invocation_consumptions t",
        ),
        "agent_instance_execution_invocations" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM agent_instance_execution_invocations t",
        ),
        "agent_instances" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM agent_instances t",
        ),
        "agent_instance_state_volumes" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM agent_instance_state_volumes t",
        ),
        "agent_instance_volume_leases" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM agent_instance_volume_leases t",
        ),
        "agent_instance_revision_volume_bindings" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM agent_instance_revision_volume_bindings t",
        ),
        "agent_instance_volume_mount_grants" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM agent_instance_volume_mount_grants t",
        ),
        "agent_instance_volume_mount_revocations" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM agent_instance_volume_mount_revocations t",
        ),
        "run_cleanup_targets" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM run_cleanup_targets t",
        ),
        "run_cleanup_receipts" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM run_cleanup_receipts t",
        ),
        "run_cleanup_target_leases" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM run_cleanup_target_leases t",
        ),
        "run_cleanup_completions" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM run_cleanup_completions t",
        ),
        "recipe_deployments" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM recipe_deployments t",
        ),
        "recipe_deployment_resources" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM recipe_deployment_resources t",
        ),
        "recipe_deployment_commands" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM recipe_deployment_commands t",
        ),
        "recipe_effect_attempts" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM recipe_effect_attempts t",
        ),
        "recipe_effect_verifications" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM recipe_effect_verifications t",
        ),
        "recipe_execution_transitions" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM recipe_execution_transitions t",
        ),
        "recipe_deployment_execution_preparations" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM recipe_deployment_execution_preparations t",
        ),
        "recipe_prepared_instance_imports" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM recipe_prepared_instance_imports t",
        ),
        "recipe_remove_resume_admissions" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM recipe_remove_resume_admissions t",
        ),
        "agent_instance_removal_requests" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM agent_instance_removal_requests t",
        ),
        "release_command_inbox" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM release_command_inbox t",
        ),
        "command_inbox" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM command_inbox t",
        ),
        "application_events" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM application_events t",
        ),
        "product_event_outbox" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM product_event_outbox t",
        ),
        "outbox" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM outbox t",
        ),
        "authorization_audit_events" => sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM authorization_audit_events t",
        ),
        _ => unreachable!("snapshot table is outside the closed whitelist"),
    }
}
