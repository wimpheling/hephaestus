use super::*;

pub(super) async fn assert_replay(
    pool: &PgPool,
    service: &ReleaseService,
    scenario: &PreparedBrokerRules,
    valid_update_id: AgentUpdateId,
    valid_candidate_revision: AgentInstanceRevisionId,
    candidate_rules: Vec<BrokeredRuleCopy>,
) {
    let actor = &scenario.actor;
    let instance_id = scenario.instance_id;
    let revision = scenario.revision;
    let source_model_rule_id = scenario.source_model_rule_id;
    let source_relay_rule_id = scenario.source_relay_rule_id;
    let candidate_agent_id = scenario.candidate_agent_id;
    let valid_command_key = key("broker-copy-valid", valid_update_id.as_uuid());
    let counts_before_replay: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM agent_instance_revisions WHERE instance_id = $1),
                (SELECT count(*) FROM brokered_secret_rules WHERE instance_revision_id = $2)",
    )
    .bind(instance_id.as_uuid())
    .bind(valid_candidate_revision.as_uuid())
    .fetch_one(pool)
    .await
    .expect("row counts before replay");
    let replay = service
        .create_update(
            actor,
            CreateInstanceUpdate {
                command_key: valid_command_key,
                update_id: AgentUpdateId::new(),
                instance_id,
                expected_revision_id: revision,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: candidate_agent_id,
                parameters: broker_copy_parameters(
                    "warning",
                    source_model_rule_id,
                    source_relay_rule_id,
                ),
                brokered_rule_copies: candidate_rules,
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("idempotent broker-rule copy replay");
    assert_eq!(replay, valid_update_id);
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM agent_instance_revisions WHERE instance_id = $1),
                (SELECT count(*) FROM brokered_secret_rules WHERE instance_revision_id = $2)",
    )
    .bind(instance_id.as_uuid())
    .bind(valid_candidate_revision.as_uuid())
    .fetch_one(pool)
    .await
    .expect("idempotent row counts");
    assert_eq!(counts, counts_before_replay);
    assert_eq!(counts.1, 2);
}
