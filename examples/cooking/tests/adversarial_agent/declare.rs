// Reuse the adversarial facade imports across the focused phases.
#[allow(unused_imports)]
use super::*;
pub(crate) async fn declare_final_rules(
    context: &CookingBuildContext<'_>,
    final_revision_id: Uuid,
    model_rule_id: Uuid,
    relay_rule_id: Uuid,
    operation_suffix: &str,
) -> Result<(Uuid, Uuid), cooking_builds::BuildError> {
    let final_bindings: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT slot_key, id
           FROM agent_secret_bindings
          WHERE instance_revision_id = $1 AND status = 'active'
          ORDER BY slot_key",
    )
    .bind(final_revision_id)
    .fetch_all(context.pool)
    .await?;
    if final_bindings.len() != 2 {
        return Err(invalid("final adversarial bindings are incomplete"));
    }
    let final_model_binding_id = final_bindings
        .iter()
        .find(|(slot, _)| slot == "model")
        .map(|(_, id)| *id)
        .ok_or_else(|| invalid("final model binding is missing"))?;
    let final_relay_binding_id = final_bindings
        .iter()
        .find(|(slot, _)| slot == "telegram_relay")
        .map(|(_, id)| *id)
        .ok_or_else(|| invalid("final relay binding is missing"))?;
    let service = SecretService::new(
        context.pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .map_err(|_| invalid("adversarial secret key setup failed"))?,
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    declare_rule(
        &service,
        context,
        final_model_binding_id,
        model_rule_id,
        "https://api.model.example",
        &format!("{operation_suffix}-declare-model-rule"),
    )
    .await?;
    declare_rule(
        &service,
        context,
        final_relay_binding_id,
        relay_rule_id,
        "https://relay.cooking.example",
        &format!("{operation_suffix}-declare-relay-rule"),
    )
    .await?;
    let final_rules: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT rule.id, rule.binding_id, binding.slot_key
           FROM brokered_secret_rules AS rule
           JOIN agent_secret_bindings AS binding
             ON binding.id = rule.binding_id
            AND binding.instance_revision_id = rule.instance_revision_id
          WHERE rule.instance_revision_id = $1
          ORDER BY binding.slot_key",
    )
    .bind(final_revision_id)
    .fetch_all(context.pool)
    .await?;
    if final_rules
        != vec![
            (model_rule_id, final_model_binding_id, String::from("model")),
            (
                relay_rule_id,
                final_relay_binding_id,
                String::from("telegram_relay"),
            ),
        ]
    {
        return Err(invalid(
            "final adversarial revision has incorrect brokered rule joins",
        ));
    }
    let stored_parameters: serde_json::Value =
        sqlx::query_scalar("SELECT parameters FROM agent_instance_revisions WHERE id = $1")
            .bind(final_revision_id)
            .fetch_one(context.pool)
            .await?;
    let model_rule_value = model_rule_id.to_string();
    let relay_rule_value = relay_rule_id.to_string();
    if stored_parameters
        .get("model_rule_id")
        .and_then(|value| value.as_str())
        != Some(model_rule_value.as_str())
        || stored_parameters
            .get("relay_rule_id")
            .and_then(|value| value.as_str())
            != Some(relay_rule_value.as_str())
    {
        return Err(invalid(
            "final adversarial parameters do not reference selected rules",
        ));
    }
    Ok((final_model_binding_id, final_relay_binding_id))
}
