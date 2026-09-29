use serde_json::Value;
use uuid::Uuid;

use super::Context;
use crate::support::{hex_hash, outbox_count, request, seed_source, seed_valid_ui};

pub async fn run(context: &Context) {
    let valid_commit = "a".repeat(40);
    let valid_receive = Uuid::new_v4();
    seed_source(
        &context.bootstrap,
        context.repository,
        valid_receive,
        &valid_commit,
        &context.configuration,
        &context.source_hash,
        &context.normalized_config_hash,
    )
    .await;
    seed_valid_ui(
        &context.bootstrap,
        context.repository,
        valid_receive,
        &valid_commit,
        context.ui_hash,
    )
    .await;
    let derived_hash = agent_config::build_identity::ui_build_definition_hash(
        context.base_hash,
        context.ui_hash,
        None,
    );
    let build_id = initial_request(context, &valid_commit, derived_hash).await;
    repeated_request(context, &valid_commit, derived_hash, build_id).await;
    concurrent_request(context, &valid_commit, derived_hash, build_id).await;
    verify_stored_build(context, &valid_commit, derived_hash, build_id).await;
}

async fn initial_request(context: &Context, commit: &str, derived_hash: [u8; 32]) -> Uuid {
    let first = context
        .application
        .request_build(
            &context.identity,
            request(
                context.repository,
                commit,
                context.base_hash,
                &context.normalized_config_hash,
            ),
        )
        .await
        .expect("base hash accepted for valid UI");
    let second = context
        .application
        .request_build(
            &context.duplicate_identity,
            request(
                context.repository,
                commit,
                derived_hash,
                &context.normalized_config_hash,
            ),
        )
        .await
        .expect("derived hash accepted for valid UI");
    assert_eq!(first.id, second.id, "base and derived callers deduplicate");
    let receipt_events: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM application_events
         WHERE occurrence_id = $1 AND aggregate_type = 'build'
           AND aggregate_id = $2
           AND scope_kind = 'repository' AND scope_id = $3 AND actor_id = $4",
    )
    .bind(context.duplicate_identity.idempotency_id.as_uuid())
    .bind(first.id)
    .bind(context.repository)
    .bind(context.owner.as_uuid())
    .fetch_one(&context.bootstrap)
    .await
    .expect("deduplicated build receipt event");
    assert_eq!(receipt_events, 1);
    first.id
}

async fn repeated_request(context: &Context, commit: &str, derived_hash: [u8; 32], build_id: Uuid) {
    let retried = context
        .application
        .request_build(
            &context.duplicate_identity,
            request(
                context.repository,
                commit,
                derived_hash,
                &context.normalized_config_hash,
            ),
        )
        .await
        .expect("repeated deduplicated build request");
    assert_eq!(retried.id, build_id);
    let receipt_events: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM application_events
         WHERE occurrence_id = $1 AND aggregate_type = 'build'
           AND aggregate_id = $2
           AND scope_kind = 'repository' AND scope_id = $3 AND actor_id = $4",
    )
    .bind(context.duplicate_identity.idempotency_id.as_uuid())
    .bind(build_id)
    .bind(context.repository)
    .bind(context.owner.as_uuid())
    .fetch_one(&context.bootstrap)
    .await
    .expect("repeated deduplicated build receipt count");
    assert_eq!(receipt_events, 1);
    assert_eq!(outbox_count(&context.bootstrap, commit).await, 1);
}

async fn concurrent_request(
    context: &Context,
    commit: &str,
    derived_hash: [u8; 32],
    build_id: Uuid,
) {
    let (first, second) = tokio::join!(
        context.application.request_build(
            &context.parallel_identity,
            request(
                context.repository,
                commit,
                derived_hash,
                &context.normalized_config_hash,
            ),
        ),
        context.application.request_build(
            &context.parallel_identity,
            request(
                context.repository,
                commit,
                derived_hash,
                &context.normalized_config_hash,
            ),
        ),
    );
    assert_eq!(
        first
            .expect("first concurrent deduplicated build request")
            .id,
        build_id
    );
    assert_eq!(
        second
            .expect("second concurrent deduplicated build request")
            .id,
        build_id
    );
    let receipt_events: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM application_events
         WHERE occurrence_id = $1 AND aggregate_type = 'build'
           AND aggregate_id = $2
           AND scope_kind = 'repository' AND scope_id = $3 AND actor_id = $4",
    )
    .bind(context.parallel_identity.idempotency_id.as_uuid())
    .bind(build_id)
    .bind(context.repository)
    .bind(context.owner.as_uuid())
    .fetch_one(&context.bootstrap)
    .await
    .expect("concurrent deduplicated build receipt count");
    assert_eq!(receipt_events, 1);
    assert_eq!(outbox_count(&context.bootstrap, commit).await, 1);
}

async fn verify_stored_build(
    context: &Context,
    commit: &str,
    derived_hash: [u8; 32],
    build_id: Uuid,
) {
    let stored_hash: Vec<u8> =
        sqlx::query_scalar("SELECT build_definition_hash FROM build_requests WHERE id = $1")
            .bind(build_id)
            .fetch_one(&context.bootstrap)
            .await
            .expect("stored derived build hash");
    assert_eq!(stored_hash, derived_hash);
    let link: (Uuid, Uuid, String, String) = sqlx::query_as(
        "SELECT build_request_id, repository_id, source_commit, source_status
         FROM build_request_ui_source_manifests WHERE build_request_id = $1",
    )
    .bind(build_id)
    .fetch_one(&context.bootstrap)
    .await
    .expect("exact UI build link");
    assert_eq!(
        link,
        (
            build_id,
            context.repository,
            commit.to_owned(),
            "valid".to_owned()
        )
    );
    let valid_event: Value = sqlx::query_scalar(
        "SELECT payload
         FROM outbox
         WHERE payload->>'source_commit' = $1
         ORDER BY occurred_at, id
         LIMIT 1",
    )
    .bind(commit)
    .fetch_one(&context.bootstrap)
    .await
    .expect("valid build event");
    assert_eq!(outbox_count(&context.bootstrap, commit).await, 1);
    let expected_hash = hex_hash(derived_hash);
    let build_id_string = build_id.to_string();
    assert_eq!(
        valid_event["build_request_id"].as_str(),
        Some(build_id_string.as_str())
    );
    assert_eq!(
        valid_event["build_definition_hash"].as_str(),
        Some(expected_hash.as_str())
    );
}
