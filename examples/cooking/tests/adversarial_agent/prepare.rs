// Reuse the adversarial facade imports across the focused phases.
#[allow(unused_imports)]
use super::*;
/// Imports and authorizes a separate adversarial instance.
///
/// Rule IDs are selected before `ImportAgent`, so the immutable imported
/// parameter document already refers to them. Each `BindSecret` revision
/// copies the previous active bindings to fresh IDs; after the second bind we
/// therefore read the final revision's binding IDs and declare the rules
/// through the authenticated service-level API on those exact bindings. No
/// later `ReviseInstance` is performed because that would clone the bindings
/// again without cloning their immutable rules.
pub(crate) async fn prepare_adversarial_instance(
    context: &CookingBuildContext<'_>,
    release_agent_id: Uuid,
    blog_repository_id: RepositoryId,
    canonical_instance_revision_id: Uuid,
    name: &str,
    operation_suffix: &str,
) -> Result<(PreparedCookingInstance, Uuid), cooking_builds::BuildError> {
    let prepared = prepare_brokered_instance(
        context,
        release_agent_id,
        blog_repository_id,
        canonical_instance_revision_id,
        name,
        operation_suffix,
    )
    .await?;
    Ok((prepared.instance, prepared.model.rule_id))
}

/// Prepares an imported instance with a caller-specific operation namespace
/// and returns both final broker rule specs. The canonical secret imports are
/// deliberately reused; this creates no additional aliases, roles, or inbound
/// credentials for a crash branch.
pub(crate) async fn prepare_brokered_instance(
    context: &CookingBuildContext<'_>,
    release_agent_id: Uuid,
    blog_repository_id: RepositoryId,
    canonical_instance_revision_id: Uuid,
    name: &str,
    operation_suffix: &str,
) -> Result<PreparedBrokeredInstance, cooking_builds::BuildError> {
    prepare_brokered_instance_with_rule_ids(
        context,
        release_agent_id,
        blog_repository_id,
        canonical_instance_revision_id,
        name,
        operation_suffix,
        BrokeredRuleIds {
            model: Uuid::new_v4(),
            relay: Uuid::new_v4(),
        },
    )
    .await
}

pub(crate) async fn canonical_brokered_secrets(
    context: &CookingBuildContext<'_>,
    canonical_instance_revision_id: Uuid,
) -> Result<CanonicalBrokeredSecrets, cooking_builds::BuildError> {
    let model_import: Uuid = sqlx::query_scalar(
        "SELECT import_id FROM agent_secret_bindings
          WHERE instance_revision_id = $1 AND slot_key = 'model'",
    )
    .bind(canonical_instance_revision_id)
    .fetch_one(context.pool)
    .await?;
    let canonical_versions: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT rule.secret_version_id, binding.slot_key
           FROM brokered_secret_rules rule
           JOIN agent_secret_bindings binding ON binding.id = rule.binding_id
          WHERE rule.instance_revision_id = $1
          ORDER BY binding.slot_key",
    )
    .bind(canonical_instance_revision_id)
    .fetch_all(context.pool)
    .await?;
    let model_version = canonical_versions
        .iter()
        .find(|(_, slot)| slot == "model")
        .map(|(version, _)| *version)
        .ok_or_else(|| invalid("canonical model rule version is missing"))?;
    let relay_version = canonical_versions
        .iter()
        .find(|(_, slot)| slot == "telegram_relay")
        .map(|(version, _)| *version)
        .ok_or_else(|| invalid("canonical relay rule version is missing"))?;
    let relay_import: Uuid = sqlx::query_scalar(
        "SELECT import_id FROM agent_secret_bindings
          WHERE instance_revision_id = $1 AND slot_key = 'telegram_relay'",
    )
    .bind(canonical_instance_revision_id)
    .fetch_one(context.pool)
    .await?;
    Ok(CanonicalBrokeredSecrets {
        model_import,
        relay_import,
        model_version,
        relay_version,
    })
}

/// Variant used by a shared upstream registry when the caller needs stable
/// rule identities across preparation and listener registration.
pub(crate) async fn prepare_brokered_instance_with_rule_ids(
    context: &CookingBuildContext<'_>,
    release_agent_id: Uuid,
    blog_repository_id: RepositoryId,
    canonical_instance_revision_id: Uuid,
    name: &str,
    operation_suffix: &str,
    rule_ids: BrokeredRuleIds,
) -> Result<PreparedBrokeredInstance, cooking_builds::BuildError> {
    let BrokeredRuleIds {
        model: model_rule_id,
        relay: relay_rule_id,
    } = rule_ids;
    let CanonicalBrokeredSecrets {
        model_import,
        relay_import,
        model_version: canonical_model_version,
        relay_version: canonical_relay_version,
    } = canonical_brokered_secrets(context, canonical_instance_revision_id).await?;
    // Import accepts only typed parameter values. The service-level rule API
    // accepts caller-generated immutable IDs, so choose those IDs first and
    // put them in the imported revision's typed parameter document.
    let instance = cooking_builds::prepare_cooking_instance_variant(
        context,
        release_agent_id,
        blog_repository_id,
        parameters(model_rule_id, relay_rule_id),
        name,
        operation_suffix,
    )
    .await?;

    // The model slot receives its existing model import and declared model
    // origin. The adversarial guest emits relay.cooking.example.
    let model = bind_secret(
        context,
        BindSecretInput {
            instance_id: instance.instance_id,
            expected_revision_id: instance.revision_id,
            import_id: model_import,
            attachment_id: instance.attachment_id,
            slot: "model",
            destinations: &["api.model.example"],
            operation: &format!("{operation_suffix}-bind-model"),
        },
    )
    .await?;
    let relay = bind_secret(
        context,
        BindSecretInput {
            instance_id: instance.instance_id,
            expected_revision_id: model.instance_revision_id,
            import_id: relay_import,
            attachment_id: instance.attachment_id,
            slot: "telegram_relay",
            destinations: &["relay.cooking.example"],
            operation: &format!("{operation_suffix}-bind-relay"),
        },
    )
    .await?;
    let final_revision_id = relay.instance_revision_id;
    let (model_binding_id, relay_binding_id) = declare_final_rules(
        context,
        final_revision_id,
        model_rule_id,
        relay_rule_id,
        operation_suffix,
    )
    .await?;
    Ok(PreparedBrokeredInstance {
        instance: PreparedCookingInstance {
            revision_id: final_revision_id,
            ..instance
        },
        model: BrokeredRuleSpec {
            rule_id: model_rule_id,
            binding_id: model_binding_id,
            instance_revision_id: final_revision_id,
            secret_version_id: canonical_model_version,
            slot: "model",
            destination: "api.model.example",
        },
        relay: BrokeredRuleSpec {
            rule_id: relay_rule_id,
            binding_id: relay_binding_id,
            instance_revision_id: final_revision_id,
            secret_version_id: canonical_relay_version,
            slot: "telegram_relay",
            destination: "relay.cooking.example",
        },
    })
}
