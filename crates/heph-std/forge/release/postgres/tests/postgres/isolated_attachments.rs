use super::*;

/// Verifies attachment isolation, tombstones, revocation, and durable history.
///
/// # Panics
/// Panics when a phase fixture operation or durable assertion fails.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn verify(context: IsolatedAttachmentContext) {
    let IsolatedAttachmentContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        first_instance,
        second_instance,
        update_release_agent,
        update_release_id,
        resume_candidate,
        ..
    } = context;
    let first_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-first", first_attachment.as_uuid()),
                attachment_id: first_attachment,
                instance_id: first_instance,
                repository_id: fixture.first_repository,
                ref_selector: RefSelector::parse("refs/heads/main")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::Push,
            },
        )
        .await
        .expect("same-project attachment should succeed");
    let historical_run = RunId::new();
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, instance_id, instance_revision_id, release_id,
          release_agent_id, attachment_id, run_kind, state, outcome,
          exit_code, requires_state, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'normal', 'cleaned_up',
                 'succeeded', 0, true, now(), now())",
    )
    .bind(historical_run.as_uuid())
    .bind(Uuid::new_v4())
    .bind(first_instance.as_uuid())
    .bind(resume_candidate.as_uuid())
    .bind(update_release_id)
    .bind(update_release_agent.as_uuid())
    .bind(first_attachment.as_uuid())
    .execute(&pool)
    .await
    .expect("historical normal run");
    let second_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-second", second_attachment.as_uuid()),
                attachment_id: second_attachment,
                instance_id: second_instance,
                repository_id: fixture.second_repository,
                ref_selector: RefSelector::parse("refs/heads/release/*")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::PushAndManual,
            },
        )
        .await
        .expect("second same-project attachment should succeed");
    let attachment_isolation: (i64, i64) = sqlx::query_as(
        "SELECT count(DISTINCT attachment.id)::bigint,
                count(DISTINCT instance.state_volume_id)::bigint
         FROM agent_attachments AS attachment
         JOIN agent_instances AS instance ON instance.id = attachment.instance_id
         WHERE attachment.instance_id = $1",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("shared instance state across attachments");
    assert_eq!(
        attachment_isolation,
        (2, 1),
        "two attachments of one instance must share its one state volume"
    );
    let cross_project_attachment = AgentAttachmentId::new();
    let cross_project = service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-cross", Uuid::new_v4()),
                attachment_id: cross_project_attachment,
                instance_id: first_instance,
                repository_id: fixture.second_repository,
                ref_selector: RefSelector::parse("refs/heads/main")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::Manual,
            },
        )
        .await;
    assert!(cross_project.is_err());
    let rolled_back_messages: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE aggregate_id = $1")
            .bind(cross_project_attachment.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("rolled-back command has no message");
    assert_eq!(rolled_back_messages, 0);
    service
        .set_attachment_enabled(
            &actor,
            SetAttachmentEnabled {
                command_key: key("disable-first", first_attachment.as_uuid()),
                attachment_id: first_attachment,
                enabled: false,
            },
        )
        .await
        .expect("authorized attachment disable");
    let disabled: bool =
        sqlx::query_scalar("SELECT NOT enabled FROM agent_attachments WHERE id = $1")
            .bind(first_attachment.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("disabled attachment");
    assert!(disabled);
    service
        .remove_attachment(
            &actor,
            RemoveAttachment {
                command_key: key("remove-first", first_attachment.as_uuid()),
                attachment_id: first_attachment,
            },
        )
        .await
        .expect("authorized attachment tombstone");
    let tombstoned: bool = sqlx::query_scalar(
        "SELECT removed_at IS NOT NULL AND NOT enabled
         FROM agent_attachments WHERE id = $1",
    )
    .bind(first_attachment.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("tombstoned attachment");
    assert!(tombstoned);
    let attachment_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
           AND event_type = 'agent_instance.changed'",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical attachment invalidations");
    assert!(attachment_events >= 3);
    service
        .revoke(
            &actor,
            key("revoke-release", update_release_id),
            ReleaseId::from_uuid(update_release_id),
        )
        .await
        .expect("authorized release revocation");
    service
        .revoke(
            &actor,
            key("revoke-release", update_release_id),
            ReleaseId::from_uuid(update_release_id),
        )
        .await
        .expect("release revocation replay is idempotent");
    let preserved_history: (Uuid, Uuid, Uuid, Uuid, bool, String) = sqlx::query_as(
        "SELECT run.id, revision.id, release_agent.id, release.id,
                attachment.removed_at IS NOT NULL, release.state
         FROM runs AS run
         JOIN agent_instance_revisions AS revision
           ON revision.id = run.instance_revision_id
          AND revision.instance_id = run.instance_id
         JOIN release_agents AS release_agent
           ON release_agent.id = run.release_agent_id
          AND release_agent.release_id = run.release_id
         JOIN releases AS release ON release.id = run.release_id
         JOIN agent_attachments AS attachment
           ON attachment.id = run.attachment_id
          AND attachment.instance_id = run.instance_id
         WHERE run.id = $1",
    )
    .bind(historical_run.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("historical foreign-key targets after tombstones");
    assert_eq!(
        preserved_history,
        (
            historical_run.as_uuid(),
            resume_candidate.as_uuid(),
            update_release_agent.as_uuid(),
            update_release_id,
            true,
            String::from("revoked"),
        )
    );
    let revocation_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'release' AND aggregate_id = $1
           AND event_type = 'release.changed' AND safe_state = 'revoked'",
    )
    .bind(update_release_id)
    .fetch_one(&pool)
    .await
    .expect("one release revocation event");
    assert_eq!(revocation_events, 1);

    let artifact_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_artifacts WHERE release_id = $1")
            .bind(release_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("artifact count");
    assert_eq!(artifact_count, 1);
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_id IN ($1, $2, $3)",
    )
    .bind(release_id.as_uuid())
    .bind(first_instance.as_uuid())
    .bind(second_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("durable event count");
    assert!(event_count >= 4);
}
