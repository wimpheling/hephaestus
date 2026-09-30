use super::*;

#[path = "updates/replay.rs"]
mod replay;

pub(super) async fn assert_updates(
    pool: &PgPool,
    service: &ReleaseService,
    scenario: &PreparedBrokerRules,
) {
    assert_invalid_mappings(pool, service, scenario).await;
    let (valid_update_id, valid_candidate_revision, candidate_rules) =
        create_valid_update(service, scenario).await;
    assert_candidate_state(pool, scenario, valid_candidate_revision, &candidate_rules).await;
    replay::assert_replay(
        pool,
        service,
        scenario,
        valid_update_id,
        valid_candidate_revision,
        candidate_rules,
    )
    .await;
}

async fn assert_invalid_mappings(
    pool: &PgPool,
    service: &ReleaseService,
    scenario: &PreparedBrokerRules,
) {
    let actor = &scenario.actor;
    let instance_id = scenario.instance_id;
    let revision = scenario.revision;
    let source_model_rule_id = scenario.source_model_rule_id;
    let source_relay_rule_id = scenario.source_relay_rule_id;
    let source_rules = &scenario.source_rules;
    let candidate_agent_id = scenario.candidate_agent_id;
    for (label, copies) in [
        ("missing", Vec::new()),
        (
            "foreign",
            vec![BrokeredRuleCopy {
                source_rule_id: Uuid::new_v4(),
                candidate_rule_id: Uuid::new_v4(),
            }],
        ),
        (
            "duplicate",
            vec![
                BrokeredRuleCopy {
                    source_rule_id: source_rules[0].0,
                    candidate_rule_id: Uuid::new_v4(),
                },
                BrokeredRuleCopy {
                    source_rule_id: source_rules[0].0,
                    candidate_rule_id: Uuid::new_v4(),
                },
            ],
        ),
    ] {
        let update_id = AgentUpdateId::new();
        let candidate_revision = AgentInstanceRevisionId::new();
        service
            .create_update(
                actor,
                CreateInstanceUpdate {
                    command_key: key(&format!("broker-copy-{label}"), update_id.as_uuid()),
                    update_id,
                    instance_id,
                    expected_revision_id: revision,
                    candidate_revision_id: candidate_revision,
                    candidate_release_agent_id: candidate_agent_id,
                    parameters: broker_copy_parameters(
                        "warning",
                        source_model_rule_id,
                        source_relay_rule_id,
                    ),
                    brokered_rule_copies: copies,
                    selected_policy: selected_policy(),
                    platform_policy: platform_policy(),
                    platform_policy_version: String::from("platform/v1"),
                },
            )
            .await
            .expect("invalid mapping remains a durable rejected update");
        let state: (String, bool) = sqlx::query_as(
            "SELECT state,
                    COALESCE(runnable, false)
             FROM agent_updates
             JOIN agent_instance_revisions revision
               ON revision.id = agent_updates.candidate_revision_id
             WHERE agent_updates.id = $1",
        )
        .bind(update_id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("invalid mapping state");
        assert_eq!(state, (String::from("rejected"), false));
    }
}

async fn create_valid_update(
    service: &ReleaseService,
    scenario: &PreparedBrokerRules,
) -> (
    AgentUpdateId,
    AgentInstanceRevisionId,
    Vec<BrokeredRuleCopy>,
) {
    let actor = &scenario.actor;
    let instance_id = scenario.instance_id;
    let revision = scenario.revision;
    let source_model_rule_id = scenario.source_model_rule_id;
    let source_relay_rule_id = scenario.source_relay_rule_id;
    let source_rules = &scenario.source_rules;
    let candidate_agent_id = scenario.candidate_agent_id;
    let candidate_rules = source_rules
        .iter()
        .map(|(source_rule_id, _)| BrokeredRuleCopy {
            source_rule_id: *source_rule_id,
            candidate_rule_id: Uuid::new_v4(),
        })
        .collect::<Vec<_>>();
    let valid_update_id = AgentUpdateId::new();
    let valid_candidate_revision = AgentInstanceRevisionId::new();
    let valid_command_key = key("broker-copy-valid", valid_update_id.as_uuid());
    service
        .create_update(
            actor,
            CreateInstanceUpdate {
                command_key: valid_command_key,
                update_id: valid_update_id,
                instance_id,
                expected_revision_id: revision,
                candidate_revision_id: valid_candidate_revision,
                candidate_release_agent_id: candidate_agent_id,
                parameters: broker_copy_parameters(
                    "warning",
                    source_model_rule_id,
                    source_relay_rule_id,
                ),
                brokered_rule_copies: candidate_rules.clone(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("complete explicit broker-rule copy update");
    (valid_update_id, valid_candidate_revision, candidate_rules)
}

async fn assert_candidate_state(
    pool: &PgPool,
    scenario: &PreparedBrokerRules,
    valid_candidate_revision: AgentInstanceRevisionId,
    candidate_rules: &[BrokeredRuleCopy],
) {
    let revision = scenario.revision;
    let source_model_rule_id = scenario.source_model_rule_id;
    let source_relay_rule_id = scenario.source_relay_rule_id;
    let source_rules_before = &scenario.source_rules_before;
    let active_versions = &scenario.active_versions;
    let candidate_parameters: (serde_json::Value, Vec<Uuid>) = sqlx::query_as(
        "SELECT revision.parameters,
                ARRAY(SELECT binding.id FROM agent_secret_bindings binding
                      WHERE binding.instance_revision_id = revision.id
                      ORDER BY binding.slot_key)
         FROM agent_instance_revisions revision
         WHERE revision.id = $1",
    )
    .bind(valid_candidate_revision.as_uuid())
    .fetch_one(pool)
    .await
    .expect("candidate parameters and bindings");
    assert_eq!(
        candidate_parameters.0,
        json!({
            "severity": "warning",
            "model_rule_id": source_model_rule_id.to_string(),
            "relay_rule_id": source_relay_rule_id.to_string(),
        })
    );
    assert_eq!(candidate_parameters.1.len(), 2);
    let stored_rules = sqlx::query_as::<_, BrokerRuleSnapshot>(
        "SELECT id, binding_id, instance_revision_id, secret_version_id,
                destination_origin, location_kind, header_name, header_prefix,
                normalized_hash
         FROM brokered_secret_rules
         WHERE instance_revision_id = $1 ORDER BY id",
    )
    .bind(valid_candidate_revision.as_uuid())
    .fetch_all(pool)
    .await
    .expect("candidate broker rules");
    assert_eq!(stored_rules.len(), 2);
    let source_bindings: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, slot_key FROM agent_secret_bindings
         WHERE instance_revision_id = $1 ORDER BY slot_key",
    )
    .bind(revision.as_uuid())
    .fetch_all(pool)
    .await
    .expect("source bindings");
    let candidate_bindings: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, slot_key FROM agent_secret_bindings
         WHERE instance_revision_id = $1 ORDER BY slot_key",
    )
    .bind(valid_candidate_revision.as_uuid())
    .fetch_all(pool)
    .await
    .expect("candidate bindings");
    assert_eq!(source_bindings.len(), 2);
    assert_eq!(candidate_bindings.len(), 2);
    assert_candidate_rule_copies(
        source_rules_before,
        active_versions,
        candidate_rules,
        &stored_rules,
        &source_bindings,
        &candidate_bindings,
        valid_candidate_revision,
    );
    assert!(
        stored_rules
            .iter()
            .all(|rule| candidate_parameters.1.contains(&rule.binding_id))
    );
    let source_rules_after = sqlx::query_as::<_, BrokerRuleSnapshot>(
        "SELECT id, binding_id, instance_revision_id, secret_version_id,
                destination_origin, location_kind, header_name, header_prefix,
                normalized_hash
         FROM brokered_secret_rules
         WHERE instance_revision_id = $1 ORDER BY id",
    )
    .bind(revision.as_uuid())
    .fetch_all(pool)
    .await
    .expect("source broker rules after update");
    assert_eq!(source_rules_after.as_slice(), source_rules_before);
}

fn assert_candidate_rule_copies(
    source_rules_before: &[BrokerRuleSnapshot],
    active_versions: &BTreeMap<Uuid, Uuid>,
    candidate_rules: &[BrokeredRuleCopy],
    stored_rules: &[BrokerRuleSnapshot],
    source_bindings: &[(Uuid, String)],
    candidate_bindings: &[(Uuid, String)],
    valid_candidate_revision: AgentInstanceRevisionId,
) {
    for source in source_rules_before {
        let copy = candidate_rules
            .iter()
            .find(|copy| copy.source_rule_id == source.id)
            .expect("candidate rule mapping");
        let candidate = stored_rules
            .iter()
            .find(|rule| rule.id == copy.candidate_rule_id)
            .expect("candidate rule");
        let source_slot = source_bindings
            .iter()
            .find(|(id, _)| *id == source.binding_id)
            .map(|(_, slot)| slot)
            .expect("source rule slot");
        let candidate_binding_id = candidate_bindings
            .iter()
            .find(|(_, slot)| slot == source_slot)
            .map(|(id, _)| *id)
            .expect("candidate rule binding");
        assert_eq!(candidate.binding_id, candidate_binding_id);
        assert_eq!(
            candidate.instance_revision_id,
            valid_candidate_revision.as_uuid()
        );
        assert_eq!(candidate.secret_version_id, active_versions[&source.id]);
        let destination = ExactHttpsOrigin::parse(&source.destination_origin).expect("origin");
        let header = HeaderName::parse(&source.header_name).expect("header");
        let location = match (source.location_kind.as_str(), source.header_prefix.clone()) {
            ("outbound_header_prefix", Some(prefix)) => {
                HttpInjectionLocation::OutboundHeaderPrefix { header, prefix }
            }
            ("outbound_header_value", None) => {
                HttpInjectionLocation::OutboundHeaderValue { header }
            }
            _ => panic!("invalid source rule location"),
        };
        let expected = BrokeredSecretRule {
            id: BrokeredSecretRuleId::from_uuid(candidate.id),
            binding_id: candidate.binding_id,
            instance_revision_id: valid_candidate_revision.as_uuid(),
            secret_version_id: candidate.secret_version_id,
            destination: Some(destination),
            location,
            gateway_route_id: None,
        }
        .normalized()
        .expect("canonical candidate rule")
        .normalized_hash();
        assert_eq!(candidate.normalized_hash, expected.as_slice());
    }
}
