use super::{
    AgentInstanceRevisionId, BTreeMap, BTreeSet, BrokeredRuleCloneRow, BrokeredRuleCopy,
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
    Postgres, ReleaseServiceError, RevisionBindingRow, Transaction, Uuid, Value, json,
};

pub fn validate_brokered_rule_copies(
    source_rule_ids: &BTreeSet<Uuid>,
    copies: &[BrokeredRuleCopy],
) -> (BTreeMap<Uuid, Uuid>, Vec<Value>) {
    let mut source_copies = BTreeSet::new();
    let mut candidate_copies = BTreeSet::new();
    let mut rule_copies = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for copy in copies {
        let valid_ids = copy.source_rule_id != Uuid::nil()
            && copy.candidate_rule_id != Uuid::nil()
            && copy.source_rule_id != copy.candidate_rule_id;
        if !valid_ids {
            diagnostics.push(json!({
                "code": "brokered_rule_copy_invalid_id",
                "field": "brokered_rule_copies"
            }));
            continue;
        }
        if !source_rule_ids.contains(&copy.source_rule_id) {
            diagnostics.push(json!({
                "code": "brokered_rule_copy_source_unavailable",
                "field": "brokered_rule_copies"
            }));
            continue;
        }
        if !source_copies.insert(copy.source_rule_id)
            || !candidate_copies.insert(copy.candidate_rule_id)
        {
            diagnostics.push(json!({
                "code": "brokered_rule_copy_duplicate",
                "field": "brokered_rule_copies"
            }));
            continue;
        }
        rule_copies.insert(copy.source_rule_id, copy.candidate_rule_id);
    }
    for source_rule_id in source_rule_ids {
        if !rule_copies.contains_key(source_rule_id) {
            diagnostics.push(json!({
                "code": "brokered_rule_copy_missing",
                "field": "brokered_rule_copies"
            }));
        }
    }
    (rule_copies, diagnostics)
}

pub async fn clone_brokered_secret_rule(
    tx: &mut Transaction<'_, Postgres>,
    source: &BrokeredRuleCloneRow,
    binding_id: Uuid,
    revision_id: AgentInstanceRevisionId,
    rule_id: Uuid,
) -> Result<(), ReleaseServiceError> {
    let destination = ExactHttpsOrigin::parse(&source.destination_origin)
        .map_err(|_| ReleaseServiceError::InvalidStoredData)?;
    let header = HeaderName::parse(&source.header_name)
        .map_err(|_| ReleaseServiceError::InvalidStoredData)?;
    let location = match (source.location_kind.as_str(), source.header_prefix.clone()) {
        ("outbound_header_value", None) => HttpInjectionLocation::OutboundHeaderValue { header },
        ("outbound_header_prefix", Some(prefix)) => {
            HttpInjectionLocation::OutboundHeaderPrefix { header, prefix }
        }
        _ => return Err(ReleaseServiceError::InvalidStoredData),
    };
    let rule = BrokeredSecretRule {
        id: BrokeredSecretRuleId::from_uuid(rule_id),
        binding_id,
        instance_revision_id: revision_id.as_uuid(),
        secret_version_id: source.secret_version_id,
        destination: Some(destination),
        location,
        gateway_route_id: None,
    }
    .normalized()
    .map_err(|_| ReleaseServiceError::InvalidStoredData)?;
    let normalized_hash = rule.normalized_hash();
    sqlx::query(
        "INSERT INTO brokered_secret_rules
           (id, binding_id, instance_revision_id, secret_version_id, direction,
            destination_origin, location_kind, header_name, header_prefix,
            normalized_hash)
         VALUES ($1, $2, $3, $4, 'outbound', $5, $6, $7, $8, $9)",
    )
    .bind(rule_id)
    .bind(binding_id)
    .bind(revision_id.as_uuid())
    .bind(source.secret_version_id)
    .bind(&source.destination_origin)
    .bind(&source.location_kind)
    .bind(&source.header_name)
    .bind(&source.header_prefix)
    .bind(normalized_hash.as_slice())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub fn required_slot_diagnostics(
    schema: &Value,
    bound_slots: &std::collections::HashSet<&str>,
) -> Result<Vec<Value>, ReleaseServiceError> {
    Ok(schema
        .as_array()
        .ok_or(ReleaseServiceError::InvalidStoredData)?
        .iter()
        .filter(|slot| slot.get("required").and_then(Value::as_bool) == Some(true))
        .filter_map(|slot| slot.get("key").and_then(Value::as_str))
        .filter(|slot| !bound_slots.contains(slot))
        .map(|slot| {
            json!({
                "code": "required_secret_binding_missing",
                "field": format!("secret_slots.{slot}")
            })
        })
        .collect())
}

pub fn candidate_accepts_binding(schema: &Value, binding: &RevisionBindingRow) -> bool {
    schema.as_array().is_some_and(|slots| {
        slots.iter().any(|slot| {
            slot.get("key").and_then(Value::as_str) == Some(&binding.slot_key)
                && slot
                    .get("delivery_modes")
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value.as_str() == Some(&binding.delivery_mode))
                    })
                && slot
                    .get("phases")
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        binding
                            .phases
                            .iter()
                            .all(|phase| values.iter().any(|value| value.as_str() == Some(phase)))
                    })
        })
    })
}

pub fn update_contract_diagnostics(
    current_requires_state: bool,
    candidate_requires_state: bool,
    candidate_update_hook: Option<&Value>,
) -> Vec<Value> {
    let mut diagnostics = Vec::new();
    if current_requires_state != candidate_requires_state {
        diagnostics.push(json!({
            "code": "state_capability_change_unsupported",
            "field": "state_volume.enabled"
        }));
    }
    if current_requires_state && candidate_update_hook.is_none() {
        diagnostics.push(json!({
            "code": "stateful_update_hook_missing",
            "field": "update_hook"
        }));
    }
    diagnostics
}
