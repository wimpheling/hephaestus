use capability_domain::{RuntimeInvocation, RuntimeSessionId, RuntimeSessionIdentity};
use heph_run::{RunAuthorityManager, RunState};
use runtime_authority::LiveRunSnapshotPins;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::{fixture::Fixture, phases::pins};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn source_status_and_revoked_session_deny_without_issuing_again(pool: PgPool) {
    let mut f = Fixture::new(pool).await;
    f.advance(RunState::Provisioning).await;
    drop(f.manager.prepare(&f.run).await.unwrap());
    let recorded = f.evidence().await;
    let envelopes = f.envelopes();
    assert_withdrawn_source(&f, &recorded, &envelopes).await;
    for status in ["suspended", "disabled"] {
        sqlx::query("UPDATE users SET status=$2 WHERE id=$1")
            .bind(f.actor.maintainer.as_uuid())
            .bind(status)
            .execute(&f.pool)
            .await
            .unwrap();
        assert!(f.manager.reauthorize(&f.run).await.is_err());
        assert!(f.manager.prepare(&f.run).await.is_err());
        assert_eq!(f.evidence().await, recorded);
        assert_eq!(f.envelopes(), envelopes);
    }
    sqlx::query("UPDATE users SET status='active' WHERE id=$1")
        .bind(f.actor.maintainer.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    f.manager.reauthorize(&f.run).await.unwrap();
    f.manager.revoke_after_guest(f.run.id).await.unwrap();
    let revoked = f.evidence().await;
    assert_eq!(revoked["session"]["status"], "revoked");
    assert!(f.manager.reauthorize(&f.run).await.is_err());
    assert!(f.manager.prepare(&f.run).await.is_err());
    assert_eq!(f.evidence().await, revoked);
    assert!(f.envelopes().is_empty());
    f.worker.close().await;
}

async fn assert_withdrawn_source(
    f: &Fixture,
    recorded: &serde_json::Value,
    envelopes: &[(String, Vec<u8>)],
) {
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.actor.project)
        .bind(f.actor.maintainer.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.manager.reauthorize(&f.run).await.is_err());
    assert!(f.manager.prepare(&f.run).await.is_err());
    assert_eq!(&f.evidence().await, recorded);
    assert_eq!(f.envelopes(), envelopes);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(f.actor.project)
        .bind(f.actor.maintainer.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    f.manager.reauthorize(&f.run).await.unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn closed_preissuance_never_creates_session_or_handoff(pool: PgPool) {
    let f = Fixture::new(pool).await;
    f.manager.reauthorize(&f.run).await.unwrap();
    sqlx::query("UPDATE runs SET volume_acquisition_closed_at=now() WHERE id=$1")
        .bind(f.run.id.as_uuid())
        .execute(&f.worker)
        .await
        .unwrap();
    assert!(f.manager.reauthorize(&f.run).await.is_err());
    assert!(f.manager.prepare(&f.run).await.is_err());
    assert!(f.evidence().await["session"].is_null());
    assert!(f.envelopes().is_empty());
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn orphan_snapshot_is_not_an_absent_session_and_lookup_is_worker_only(pool: PgPool) {
    let f = Fixture::new(pool).await;
    let selected = pins(&f);
    let default = PgRuntimeSessionRepository::new(f.pool.clone());
    assert!(
        default
            .recorded_live_session_ceiling(selected)
            .await
            .is_err()
    );
    assert!(
        default
            .with_canonical_live_snapshots()
            .recorded_live_session_ceiling(selected)
            .await
            .is_err()
    );
    let snapshot = super::super::live::snapshot(&f.manager.repository, &f.run)
        .await
        .unwrap();
    sqlx::query("INSERT INTO run_authorization_snapshots(id,run_id,instance_id,instance_revision_id,authorization_model_version,normalized_hash) VALUES($1,$1,$2,$3,$4,$5)")
        .bind(f.run.id.as_uuid()).bind(f.run.instance_id.as_uuid())
        .bind(f.run.instance_revision_id.as_uuid()).bind(snapshot.authorization_model_version())
        .bind(snapshot.normalized_hash().as_bytes().as_slice()).execute(&f.worker).await.unwrap();
    let orphan = f.evidence().await;
    assert!(orphan["session"].is_null());
    assert!(!orphan["snapshot"].is_null());
    assert!(f.manager.reauthorize(&f.run).await.is_err());
    assert!(f.manager.prepare(&f.run).await.is_err());
    assert_eq!(f.evidence().await, orphan);
    assert!(f.envelopes().is_empty());
    let foreign = LiveRunSnapshotPins::new(
        selected.run(),
        selected.instance(),
        Uuid::new_v4(),
        selected.release(),
        selected.release_agent(),
        selected.command(),
    )
    .unwrap();
    assert!(
        f.manager
            .repository
            .recorded_live_session_ceiling(foreign)
            .await
            .is_err()
    );
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn expired_actual_issuer_session_never_falls_back_to_preissuance(pool: PgPool) {
    let mut f = Fixture::new(pool).await;
    f.advance(RunState::Provisioning).await;
    let snapshot = f.manager.snapshot(&f.run).await.unwrap();
    let issued_at = OffsetDateTime::now_utc() - Duration::minutes(2);
    let expires_at = issued_at + Duration::minutes(1);
    let identity = RuntimeSessionIdentity::new(
        RuntimeSessionId::from_uuid(f.run.id.as_uuid()),
        snapshot.principal(),
        RuntimeInvocation::Run(f.run.id),
        &snapshot,
        issued_at,
        expires_at,
    )
    .unwrap();
    // Use the actual issuer and encrypted store with a past issuance clock;
    // the recorded session is now elapsed, rather than forged by an UPDATE.
    drop(
        f.manager
            .issuer
            .issue(
                &snapshot,
                &identity,
                f.run
                    .attachment_id
                    .map(runtime_types::AgentAttachmentId::as_uuid),
                issued_at,
            )
            .await
            .unwrap(),
    );
    let expired = f.evidence().await;
    let envelopes = f.envelopes();
    assert!(f.manager.reauthorize(&f.run).await.is_err());
    assert!(f.manager.prepare(&f.run).await.is_err());
    assert_eq!(f.evidence().await, expired);
    assert_eq!(f.envelopes(), envelopes);
    f.worker.close().await;
}
