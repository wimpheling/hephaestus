use super::*;

pub(super) struct DeclaredSlot {
    delivery_modes: Vec<String>,
    phases: Vec<String>,
    destinations: Vec<String>,
}

pub(super) fn declared_slot(
    schema: &serde_json::Value,
    slot_key: &str,
) -> Result<DeclaredSlot, SecretServiceError> {
    let declaration = schema
        .as_array()
        .and_then(|slots| {
            slots
                .iter()
                .find(|slot| slot.get("key").and_then(serde_json::Value::as_str) == Some(slot_key))
        })
        .ok_or(SecretServiceError::SlotNotDeclared)?;
    Ok(DeclaredSlot {
        delivery_modes: string_array(declaration, "delivery_modes")?,
        phases: string_array(declaration, "phases")?,
        destinations: string_array(declaration, "destinations")?,
    })
}

pub(super) fn string_array(
    value: &serde_json::Value,
    field: &str,
) -> Result<Vec<String>, SecretServiceError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or(SecretServiceError::InvalidStoredData)?
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToOwned::to_owned)
                .ok_or(SecretServiceError::InvalidStoredData)
        })
        .collect()
}

pub(super) fn validate_declared_binding(
    declaration: &DeclaredSlot,
    command: &BindSecret,
) -> Result<(), SecretServiceError> {
    if !declaration
        .delivery_modes
        .iter()
        .any(|mode| mode == mode_name(command.mode))
        || command.phases.iter().any(|phase| {
            !declaration
                .phases
                .iter()
                .any(|value| value == phase_name(*phase))
        })
        || (!declaration.destinations.is_empty()
            && command
                .destinations
                .iter()
                .any(|destination| !declaration.destinations.contains(destination)))
    {
        return Err(SecretServiceError::BindingPolicyMismatch);
    }
    Ok(())
}

pub(super) async fn load_eligible_import(
    tx: &mut Transaction<'_, Postgres>,
    import_id: SecretImportId,
) -> Result<EligibleImportRow, SecretServiceError> {
    sqlx::query_as(
        "SELECT source_grant.id AS grant_id, secret.id AS secret_id,
                  secret.owner_organization_id, imported.target_kind,
                  imported.target_id, source_grant.delivery_modes,
                  source_grant.phases, source_grant.destinations
           FROM secret_imports AS imported
           JOIN secret_grants AS source_grant
             ON source_grant.id = imported.grant_id
           JOIN secrets AS secret ON secret.id = imported.secret_id
           WHERE imported.id = $1 AND imported.status = 'active'
             AND source_grant.status = 'active' AND secret.status = 'active'
             AND (
                 source_grant.expires_at IS NULL
                 OR source_grant.expires_at > now()
             )",
    )
    .bind(import_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| SecretServiceError::Persistence)?
    .ok_or(SecretServiceError::Unavailable)
}

pub(super) fn validate_import_policy(
    import: &EligibleImportRow,
    command: &BindSecret,
) -> Result<(), SecretServiceError> {
    if !import
        .delivery_modes
        .iter()
        .any(|mode| mode == mode_name(command.mode))
        || command.phases.iter().any(|phase| {
            !import
                .phases
                .iter()
                .any(|value| value == phase_name(*phase))
        })
        || (!import.destinations.is_empty()
            && command
                .destinations
                .iter()
                .any(|destination| !import.destinations.contains(destination)))
    {
        return Err(SecretServiceError::BindingPolicyMismatch);
    }
    Ok(())
}

pub(super) async fn validate_binding_scope(
    tx: &mut Transaction<'_, Postgres>,
    instance_id: AgentInstanceId,
    project_id: Uuid,
    import: &EligibleImportRow,
    command: &BindSecret,
) -> Result<(), SecretServiceError> {
    let includes_normal = command.phases.contains(&ExecutionPhase::Normal);
    let includes_update = command.phases.contains(&ExecutionPhase::Update);
    if includes_normal && command.attachment_ids.is_empty() {
        return Err(SecretServiceError::BindingOutOfScope);
    }
    if import.target_kind == "project" {
        if import.target_id != project_id {
            return Err(SecretServiceError::BindingOutOfScope);
        }
    } else if import.target_kind == "repository" {
        if command.attachment_ids.is_empty() || includes_update {
            return Err(SecretServiceError::BindingOutOfScope);
        }
    } else {
        return Err(SecretServiceError::InvalidStoredData);
    }
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, repository_id FROM agent_attachments
           WHERE instance_id = $1 AND id = ANY($2)
             AND enabled AND removed_at IS NULL",
    )
    .bind(instance_id.as_uuid())
    .bind(&command.attachment_ids)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    if rows.len() != command.attachment_ids.len()
        || (import.target_kind == "repository"
            && rows
                .iter()
                .any(|(_, repository_id)| *repository_id != import.target_id))
    {
        return Err(SecretServiceError::BindingOutOfScope);
    }
    Ok(())
}

pub(super) fn validate_carried_policy(
    import: &EligibleImportRow,
    binding: &CarriedBindingRow,
) -> Result<(), SecretServiceError> {
    if !import.delivery_modes.contains(&binding.delivery_mode)
        || binding
            .phases
            .iter()
            .any(|phase| !import.phases.contains(phase))
        || (!import.destinations.is_empty()
            && binding
                .destinations
                .iter()
                .any(|destination| !import.destinations.contains(destination)))
    {
        return Err(SecretServiceError::BindingPolicyMismatch);
    }
    Ok(())
}

pub(super) fn unresolved_required_diagnostics<'a>(
    schema: &serde_json::Value,
    bound_slots: impl Iterator<Item = &'a str>,
) -> Result<serde_json::Value, SecretServiceError> {
    let bound = bound_slots.collect::<std::collections::HashSet<_>>();
    let diagnostics = schema
        .as_array()
        .ok_or(SecretServiceError::InvalidStoredData)?
        .iter()
        .filter(|slot| slot.get("required").and_then(serde_json::Value::as_bool) == Some(true))
        .filter_map(|slot| slot.get("key").and_then(serde_json::Value::as_str))
        .filter(|slot| !bound.contains(slot))
        .map(|slot| {
            json!({
                "code": "required_secret_binding_missing",
                "field": format!("secret_slots.{slot}")
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::Value::Array(diagnostics))
}

pub(super) async fn insert_binding_copy(
    tx: &mut Transaction<'_, Postgres>,
    binding_id: AgentSecretBindingId,
    revision_id: AgentInstanceRevisionId,
    binding: &CarriedBindingRow,
    creator_id: Uuid,
) -> Result<(), SecretServiceError> {
    sqlx::query(
        "INSERT INTO agent_secret_bindings
           (id, instance_revision_id, import_id, slot_key, delivery_mode,
            phases, attachment_ids, destinations, effective_policy,
            effective_policy_hash, status, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                   'active', $11)",
    )
    .bind(binding_id.as_uuid())
    .bind(revision_id.as_uuid())
    .bind(binding.import_id)
    .bind(&binding.slot_key)
    .bind(&binding.delivery_mode)
    .bind(&binding.phases)
    .bind(&binding.attachment_ids)
    .bind(&binding.destinations)
    .bind(&binding.effective_policy)
    .bind(&binding.effective_policy_hash)
    .bind(creator_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    Ok(())
}
