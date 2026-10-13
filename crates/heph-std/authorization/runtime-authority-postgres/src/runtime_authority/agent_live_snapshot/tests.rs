use runtime_authority::{
    LiveRunSnapshotPins, LiveRunSnapshotRequest, LiveRunSnapshotResolver, RuntimeAuthorityError,
};
use sqlx::postgres::PgPoolOptions;
use time::OffsetDateTime;
use uuid::Uuid;

use super::PgRuntimeSessionRepository;

#[tokio::test]
async fn ordinary_constructor_rejects_live_profile_without_contacting_database() {
    // No server exists here. Default rejection must precede pool checkout and
    // any query that could reference private owned-profile schema.
    let pool = PgPoolOptions::new()
        .connect_lazy("postgresql://fixture:fixture@127.0.0.1:1/unavailable")
        .unwrap();
    let repository = PgRuntimeSessionRepository::new(pool);
    let pins = LiveRunSnapshotPins::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    )
    .unwrap();
    let result = repository
        .resolve_live_snapshot(
            &LiveRunSnapshotRequest::pre_issuance(pins),
            "test/v1",
            OffsetDateTime::now_utc(),
        )
        .await;
    assert_eq!(result.unwrap_err(), RuntimeAuthorityError::Unsupported);
}
