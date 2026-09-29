use control_plane_postgres::build::BuildError;
use uuid::Uuid;

use super::Context;
use crate::support::{build_count, outbox_count, request, seed_invalid_ui, seed_source};

pub async fn run(context: &Context) {
    let invalid_commit = "b".repeat(40);
    let invalid_receive = Uuid::new_v4();
    seed_source(
        &context.bootstrap,
        context.repository,
        invalid_receive,
        &invalid_commit,
        &context.configuration,
        &context.source_hash,
        &context.normalized_config_hash,
    )
    .await;
    seed_invalid_ui(
        &context.bootstrap,
        context.repository,
        invalid_receive,
        &invalid_commit,
    )
    .await;
    let invalid = context
        .application
        .request_build(
            &context.identity,
            request(
                context.repository,
                &invalid_commit,
                context.base_hash,
                &context.normalized_config_hash,
            ),
        )
        .await;
    assert!(matches!(invalid, Err(BuildError::FailedPrecondition)));
    assert_eq!(
        build_count(&context.bootstrap, context.repository, &invalid_commit).await,
        0
    );
    assert_eq!(outbox_count(&context.bootstrap, &invalid_commit).await, 0);
}
