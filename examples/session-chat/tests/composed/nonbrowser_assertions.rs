use super::denial::{accepted_receive_count, assert_denial_probe_output, canonical_main_ref};
use super::model::SessionBrokerFixture;
use super::nonbrowser_setup::NonBrowserState;
use super::runtime_assertions::assert_runtime_git_turn;
use super::{DENIAL_HUMAN_RECORD_ID, HUMAN_RECORD_ID, SESSION_ID};
use sqlx::PgPool;
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
// Keep the nonbrowser provenance checks together as one reviewable acceptance phase.
pub(crate) async fn verify_nonbrowser(
    pool: &PgPool,
    root: &Path,
    identity_id: Uuid,
    broker: SessionBrokerFixture,
    state: NonBrowserState,
) {
    let NonBrowserState {
        built,
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
        run_id,
        human_commit,
        denial_probe,
        denial_source_repository_id,
        denial_other_repository_id,
        accepted_before_turn,
        denial_source_ref_before,
        denial_other_ref_before,
        denial_source_accepts_before,
        denial_other_accepts_before,
    } = state;
    assert_runtime_git_turn(
        pool,
        root,
        repository_id,
        instance_id,
        attachment_id,
        identity_id,
        run_id,
        &human_commit,
        Some(if denial_probe {
            DENIAL_HUMAN_RECORD_ID
        } else {
            HUMAN_RECORD_ID
        }),
    )
    .await;
    if let (
        Some(source_id),
        Some(other_id),
        Some(source_before),
        Some(other_before),
        Some(source_ref_before),
        Some(other_ref_before),
    ) = (
        denial_source_repository_id,
        denial_other_repository_id,
        denial_source_accepts_before,
        denial_other_accepts_before,
        denial_source_ref_before,
        denial_other_ref_before,
    ) {
        assert_eq!(
            source_ref_before, built.source_commit,
            "denial source must be the published build repository"
        );
        assert!(
            !other_ref_before.is_empty(),
            "comparison repository must have a canonical main ref"
        );
        assert_denial_probe_output(pool, run_id).await;
        assert_eq!(
            accepted_receive_count(pool, repository_id).await,
            accepted_before_turn + 2,
            "denial pushes must not add an accepted target receive"
        );
        assert_eq!(
            accepted_receive_count(pool, source_id).await,
            source_before,
            "source-repository denial attempts must not be accepted"
        );
        assert_eq!(
            accepted_receive_count(pool, other_id).await,
            other_before,
            "other-repository denial attempts must not be accepted"
        );
        assert_eq!(
            canonical_main_ref(pool, source_id).await.as_deref(),
            Some(source_ref_before.as_str()),
            "source-repository canonical ref must remain unchanged"
        );
        assert_eq!(
            canonical_main_ref(pool, other_id).await.as_deref(),
            Some(other_ref_before.as_str()),
            "other-repository canonical ref must remain unchanged"
        );
        eprintln!(
            "HEPH_SESSION_CHAT_DENIAL_PROBE host=validated checks=10 refs=unchanged receives=unchanged"
        );
    }
    let (stored_input, stored_ref, stored_revision): (String, String, Uuid) = sqlx::query_as(
        "SELECT request.commit_sha, request.git_ref, run.instance_revision_id
           FROM run_requests request JOIN runs run ON run.id = request.run_id
          WHERE request.run_id = $1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("session-chat runtime provenance");
    assert_eq!(stored_input, human_commit);
    assert_eq!(stored_ref, "refs/heads/main");
    assert_ne!(stored_revision, revision_id);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let run_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(instance_id)
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("session-chat run request count");
    assert_eq!(
        run_count, 1,
        "assistant publication must not recursively trigger a run"
    );
    let observed = broker.assert_observed().await;
    if denial_probe {
        assert_eq!(
            observed
                .iter()
                .map(|request| request.session_id)
                .collect::<Vec<_>>(),
            [
                Uuid::parse_str(SESSION_ID).expect("denial-probe session UUID"),
                Uuid::parse_str(SESSION_ID).expect("denial-probe session UUID"),
            ]
        );
        assert_eq!(
            observed
                .iter()
                .map(|request| request.record_id)
                .collect::<Vec<_>>(),
            [
                Uuid::parse_str("22222222-2222-4222-8222-222222222222")
                    .expect("denial-probe control record UUID"),
                Uuid::parse_str(DENIAL_HUMAN_RECORD_ID).expect("denial-probe human record UUID"),
            ]
        );
    }
}
