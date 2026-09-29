use control_plane_postgres::build::BuildError;
use uuid::Uuid;

use super::Context;
use crate::support::{
    build_count, outbox_count, outbox_count_for_build, request, seed_source, seed_valid_ui,
};

pub async fn run(context: &Context) {
    let absent_commit = "c".repeat(40);
    let absent_receive = Uuid::new_v4();
    let absent_id = create_legacy_build(context, &absent_commit, absent_receive).await;
    let (derived_hash, derived_id) =
        capture_ui_and_derive(context, &absent_commit, absent_receive, absent_id).await;
    deduplicate_legacy_build(context, &absent_commit, derived_hash, absent_id, derived_id).await;
    reject_wrong_hash(context).await;
}

async fn create_legacy_build(context: &Context, commit: &str, receive: Uuid) -> Uuid {
    seed_source(
        &context.bootstrap,
        context.repository,
        receive,
        commit,
        &context.configuration,
        &context.source_hash,
        &context.normalized_config_hash,
    )
    .await;
    let absent = context
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
        .expect("absent UI preserves legacy identity");
    let absent_hash: Vec<u8> =
        sqlx::query_scalar("SELECT build_definition_hash FROM build_requests WHERE id = $1")
            .bind(absent.id)
            .fetch_one(&context.bootstrap)
            .await
            .expect("legacy build hash");
    assert_eq!(absent_hash, context.base_hash);
    assert_eq!(outbox_count(&context.bootstrap, commit).await, 1);
    absent.id
}

async fn capture_ui_and_derive(
    context: &Context,
    commit: &str,
    receive: Uuid,
    absent_id: Uuid,
) -> ([u8; 32], Uuid) {
    seed_valid_ui(
        &context.bootstrap,
        context.repository,
        receive,
        commit,
        context.ui_hash,
    )
    .await;
    let derived_hash = agent_config::build_identity::ui_build_definition_hash(
        context.base_hash,
        context.ui_hash,
        None,
    );
    let absent_derived = context
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
        .expect("legacy build accepts newly captured UI as a new identity");
    assert_ne!(absent_derived.id, absent_id);
    let absent_derived_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT build_definition_hash
         FROM build_requests
         WHERE id = $1",
    )
    .bind(absent_derived.id)
    .fetch_one(&context.bootstrap)
    .await
    .expect("derived legacy build hash");
    assert_eq!(absent_derived_hash, derived_hash);
    let old_identity: (Uuid, Vec<u8>) = sqlx::query_as(
        "SELECT id, build_definition_hash
         FROM build_requests
         WHERE id = $1",
    )
    .bind(absent_id)
    .fetch_one(&context.bootstrap)
    .await
    .expect("legacy build identity remains stable");
    assert_eq!(old_identity, (absent_id, context.base_hash.to_vec()));
    let old_link_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM build_request_ui_source_manifests
         WHERE build_request_id = $1",
    )
    .bind(absent_id)
    .fetch_one(&context.bootstrap)
    .await
    .expect("legacy build remains unlinked");
    assert_eq!(old_link_count, 0);
    let new_link_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM build_request_ui_source_manifests
         WHERE build_request_id = $1 AND source_status = 'valid'",
    )
    .bind(absent_derived.id)
    .fetch_one(&context.bootstrap)
    .await
    .expect("derived legacy build UI link");
    assert_eq!(new_link_count, 1);
    (derived_hash, absent_derived.id)
}

async fn deduplicate_legacy_build(
    context: &Context,
    commit: &str,
    derived_hash: [u8; 32],
    absent_id: Uuid,
    derived_id: Uuid,
) {
    let absent_derived = context
        .application
        .request_build(
            &context.identity,
            request(
                context.repository,
                commit,
                derived_hash,
                &context.normalized_config_hash,
            ),
        )
        .await
        .expect("derived legacy build deduplicates");
    assert_eq!(absent_derived.id, derived_id);
    assert_ne!(absent_derived.id, absent_id);
    assert_eq!(outbox_count(&context.bootstrap, commit).await, 2);
    assert_eq!(
        outbox_count_for_build(&context.bootstrap, absent_id).await,
        1
    );
    assert_eq!(
        outbox_count_for_build(&context.bootstrap, absent_derived.id).await,
        1
    );
}

async fn reject_wrong_hash(context: &Context) {
    let valid_commit = "a".repeat(40);
    let wrong = context
        .application
        .request_build(
            &context.identity,
            request(
                context.repository,
                &valid_commit,
                [9; 32],
                &context.normalized_config_hash,
            ),
        )
        .await;
    assert!(matches!(wrong, Err(BuildError::FailedPrecondition)));
    assert_eq!(
        build_count(&context.bootstrap, context.repository, &valid_commit).await,
        1
    );
    assert_eq!(outbox_count(&context.bootstrap, &valid_commit).await, 1);
    let valid_build_id: Uuid = sqlx::query_scalar(
        "SELECT id
         FROM build_requests
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(context.repository)
    .bind(&valid_commit)
    .fetch_one(&context.bootstrap)
    .await
    .expect("valid build identity");
    assert_eq!(
        outbox_count_for_build(&context.bootstrap, valid_build_id).await,
        1
    );
}
