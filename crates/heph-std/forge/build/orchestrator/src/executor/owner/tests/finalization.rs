use super::*;
use crate::{BuildInput, ClaimedBuild};
use release_domain::{ReleaseAgentId, ReleaseId, ReleaseVersion};

fn claimed(id: BuildRequestId) -> ClaimedBuild {
    let build = agent_config::parse(super::super::super::tests::CONFIG.as_bytes())
        .config
        .unwrap()
        .build
        .unwrap();
    ClaimedBuild {
        input: BuildInput {
            id,
            repository_id: uuid::Uuid::new_v4(),
            source_commit: "a".repeat(40),
            source_ref: "refs/heads/main".into(),
            build,
            image_reference:
                "build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
        },
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        release_version: ReleaseVersion::parse("controlled-build").unwrap(),
    }
}
#[tokio::test]
async fn sealed_failure_keeps_output_and_closed_finalization_uses_same_original_without_provision()
{
    let fixture = Fixture::new(Mode::Execute);
    let original = Arc::new(claimed(fixture.node.id));
    let node = fixture.node.clone();
    let token = fixture.token.clone();
    jobs::start(&fixture.node, {
        let original = original.clone();
        async move {
            node.claim(&original).unwrap();
            node.install_attempt(token.clone()).unwrap();
            let instance = token.provision().await.unwrap();
            node.instance(instance);
            node.detach().await.unwrap();
            node.preserve_finalization();
            Outcome::Build(Err(Arc::new(
                super::super::super::BuildExecutionError::Release,
            )))
        }
    });
    fixture.node.outcome().await;
    watcher_finished(&fixture.node).await;
    fixture.registry.close();
    fixture.node.kick_cleanup(true);
    assert!(
        observe(&[fixture.node.clone()], Duration::from_millis(40))
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(fixture.path().join("output")).unwrap(),
        b"original sealed output"
    );
    bounded(async {
        loop {
            fixture.node.join_finished_jobs().await;
            if lock(&fixture.node.cleanup_job)
                .as_ref()
                .is_none_or(|job| job.joined())
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    let previous_work = lock(&fixture.node.work_job).as_ref().unwrap().clone();
    let previous_outcome = previous_work.join(&fixture.node).await.unwrap();
    let Admission::Start(continued, Mode::Finalize) = fixture
        .registry
        .register(fixture.node.id, Mode::Finalize)
        .unwrap()
    else {
        panic!("saved finalization continuation")
    };
    assert!(Arc::ptr_eq(&continued, &fixture.node));
    jobs::finish(&continued, &previous_work, previous_outcome);
    assert!(lock(&continued.state).outcome.is_none()); // No stale original replay.

    assert!(Arc::ptr_eq(
        &continued.original_workspace().unwrap(),
        &fixture.pin
    ));
    let mut wrong = claimed(fixture.node.id);
    wrong.input = original.input.clone();
    assert!(continued.claim(&wrong).is_err());
    let result = super::super::super::BuildExecutionResult {
        build_request_id: fixture.node.id,
        release_id: original.release_id,
        release_agent_id: original.release_agent_id,
        release_version: original.release_version.clone(),
        artifact_count: 1,
    };
    let node = continued.clone();
    // Controlled repository/publication outcome, NOT actual PG authorization.
    jobs::start(&continued, async move {
        node.claim(&original).unwrap();
        node.original_workspace().unwrap().recheck().unwrap();
        node.completed_result(&result).unwrap();
        node.finalized();
        node.remove_workspace().unwrap();
        Outcome::Build(Ok(result))
    });
    continued.outcome().await;
    bounded(observe(&[continued], Duration::from_secs(1)))
        .await
        .unwrap();
    assert_eq!(fixture.token.provisions.load(Ordering::SeqCst), 1);
    assert!(!fixture.path().exists());
    assert_eq!(lock(&fixture.node.state).history.len(), 1);
}
