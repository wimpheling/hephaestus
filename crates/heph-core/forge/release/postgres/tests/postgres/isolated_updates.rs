use super::*;

/// Builds the revision/update gate state consumed by the transport phase.
///
/// # Panics
/// Panics when a fixture operation or publication assertion fails.
// This phase intentionally keeps its setup SQL and immutable publication checks together.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn prepare(context: IsolatedPublishedContext) -> IsolatedUpdateContext {
    let IsolatedPublishedContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        release_agent_id,
        first_instance,
        first_revision,
        second_instance,
    } = context;
    let revised_first = AgentInstanceRevisionId::new();
    service
        .revise_instance(
            &actor,
            ReviseInstance {
                command_key: key("revise-first", revised_first.as_uuid()),
                instance_id: first_instance,
                expected_revision_id: first_revision,
                new_revision_id: revised_first,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await
        .expect("parameter change should create an immutable revision");
    let history: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, parameters ->> 'severity'
         FROM agent_instance_revisions
         WHERE instance_id = $1 ORDER BY created_at, id",
    )
    .bind(first_instance.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("immutable revision history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].1, "warning");
    assert_eq!(history[1].1, "error");
    let stale_revision = service
        .revise_instance(
            &actor,
            ReviseInstance {
                command_key: key("stale-revise", Uuid::new_v4()),
                instance_id: first_instance,
                expected_revision_id: first_revision,
                new_revision_id: AgentInstanceRevisionId::new(),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        stale_revision,
        Err(release_postgres::ReleaseServiceError::StaleInstanceRevision)
    ));
    let broadened_revision_id = AgentInstanceRevisionId::new();
    let broadened = service
        .revise_instance(
            &actor,
            ReviseInstance {
                command_key: key("broaden-policy", broadened_revision_id.as_uuid()),
                instance_id: first_instance,
                expected_revision_id: revised_first,
                new_revision_id: broadened_revision_id,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: RuntimePolicy {
                    vcpus: 5,
                    memory_mib: 1024,
                    network: NetworkAccess::Egress,
                },
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        broadened,
        Err(release_postgres::ReleaseServiceError::Domain(
            release_domain::ReleaseValueError::PolicyBroadening
        ))
    ));
    let broadened_persisted: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM agent_instance_revisions WHERE id = $1
         )",
    )
    .bind(broadened_revision_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("inspect rejected policy revision");
    assert!(!broadened_persisted);
    let unsupported_update_id = AgentUpdateId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("unsupported-update", unsupported_update_id.as_uuid()),
                update_id: unsupported_update_id,
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: release_agent_id,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await
        .expect("unsupported candidate should remain visible");
    let unsupported: (String, bool, bool) = sqlx::query_as(
        "SELECT update.state,
                EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(update.diagnostics) AS item
                    WHERE item->>'code' = 'stateful_update_hook_missing'
                ),
                instance.run_gate_open
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(unsupported_update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("unsupported update diagnostics");
    assert_eq!(unsupported.0, "rejected");
    assert!(unsupported.1);
    assert!(unsupported.2);

    let fork_release_agent = seed_fork_release(
        &pool,
        release_id,
        release_agent_id,
        fixture.second_repository,
    )
    .await;
    let fork_update = service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("fork-update", Uuid::new_v4()),
                update_id: AgentUpdateId::new(),
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: fork_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        fork_update,
        Err(release_postgres::ReleaseServiceError::AgentFamilyMismatch)
    ));

    let update_release_agent = seed_update_release(&pool, release_id, release_agent_id).await;
    let update_release_id: Uuid =
        sqlx::query_scalar("SELECT release_id FROM release_agents WHERE id = $1")
            .bind(update_release_agent.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("candidate release");
    let deferred_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-before-update", deferred_attachment.as_uuid()),
                attachment_id: deferred_attachment,
                instance_id: first_instance,
                repository_id: fixture.first_aux_repository,
                ref_selector: RefSelector::parse("refs/heads/main")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::Push,
            },
        )
        .await
        .expect("pre-update attachment");
    let deferred_receive = Uuid::new_v4();
    let deferred_commit = "e".repeat(40);
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'deferred-test', 'accepted', now())",
    )
    .bind(deferred_receive)
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&pool)
    .await
    .expect("seed exact target receive");
    let prior_request_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, receive_id,
          run_id, command_id, instance_id, instance_revision_id,
          release_id, release_agent_id, attachment_id, request_kind,
          platform_policy_version, requires_state, dispatch_state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, $6, $7, $8,
                 $9, $10, $11, 'instance_normal', 'platform/v2', true,
                 'pending')",
    )
    .bind(prior_request_id)
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(&deferred_commit)
    .bind(deferred_receive)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(first_instance.as_uuid())
    .bind(revised_first.as_uuid())
    .bind(release_id.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind(deferred_attachment.as_uuid())
    .execute(&pool)
    .await
    .expect("seed prior exact-revision request");
    let update_id = AgentUpdateId::new();
    let update_candidate_revision = AgentInstanceRevisionId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("valid-update", update_id.as_uuid()),
                update_id,
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: update_candidate_revision,
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await
        .expect("valid stateful candidate should close the gate");
    // Reproduce the production race through both adapters: the accepted
    // mailbox dispatch is consumed while the update gate is closed, then the
    // release service must commit a fresh wake when activation reopens it.

    IsolatedUpdateContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        first_instance,
        second_instance,
        revised_first,
        update_release_agent,
        update_release_id,
        deferred_attachment,
        deferred_receive,
        prior_request_id,
        deferred_commit,
        update_id,
        update_candidate_revision,
    }
}
