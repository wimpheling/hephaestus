use crate::{
    fixture::{Fixture, snapshot},
    fixtures,
};
use identity_domain::RequestId;
use recipe_application::InstallProgress;
use sqlx::PgPool;
use volume_trait::OwnedBackingPhase;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn actual_first_birth_completes_original_claim_and_completed_retry_has_no_fsck_or_format(
    pool: PgPool,
) {
    let f = Fixture::new(&pool).await;
    let result = f.create().await.unwrap();
    assert_eq!(result.install, InstallProgress::Ready);
    assert!(result.active_attempt.is_none());
    assert_eq!((f.count("formats"), f.count("fscks")), (1, 2));
    let current = f.stored_context().await;
    let fields = current.observation.as_ref().unwrap().fields();
    assert_eq!(fields.phase, OwnedBackingPhase::Ready);
    assert!(fields.filesystem.as_ref().unwrap().clean);
    let bytes = std::fs::read(current.claim.purpose.canonical_path()).unwrap();
    assert_eq!(&bytes[1080..1082], &[0x53, 0xef]);
    assert_eq!(
        &bytes[1128..1144],
        current
            .claim
            .purpose
            .receipt()
            .intent
            .registration()
            .filesystem_uuid()
            .as_bytes()
    );
    let original = snapshot(&pool).await;
    let mut fresh = f.h.actor.clone();
    fresh.request_id = RequestId::new();
    assert_eq!(
        f.creator
            .create_prepared_volume(&fresh, f.h.command, f.h.intent.id(), &fixtures::key("data"))
            .await
            .unwrap(),
        result
    );
    assert_eq!(snapshot(&pool).await, original);
    assert_eq!(
        std::fs::read(current.claim.purpose.canonical_path()).unwrap(),
        bytes
    );
    assert_eq!((f.count("formats"), f.count("fscks")), (1, 2));
    assert!(
        f.local.verify_owned_ready(&current.claim).await.is_err(),
        "completed stale verifier cannot fsck an admitted runtime volume"
    );
    assert_eq!(f.count("fscks"), 2);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn original_pending_admission_resumes_same_operation_without_another_fence(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let (_, request) = f.request().await;
    let admitted = volume_trait::VolumeOwnedProvisioningRepository::begin_owned_provisioning(
        f.actor.as_ref(),
        &f.h.actor,
        &request,
    )
    .await
    .unwrap();
    assert!(admitted.observation.is_none());
    assert_eq!(f.count("formats"), 0);
    let result = f.create().await.unwrap();
    assert_eq!(result.install, InstallProgress::Ready);
    assert_eq!(f.stored_context().await.claim, admitted.claim);
    let operations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM volume_owned_provisioning_operations WHERE volume_id=$1",
    )
    .bind(
        request
            .purpose
            .receipt()
            .intent
            .registration()
            .id()
            .as_uuid(),
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(operations, 1);
    assert_eq!(f.count("formats"), 1);
}
