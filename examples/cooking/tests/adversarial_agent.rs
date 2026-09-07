//! Production-path cooking agent authority probe.
//!
//! This helper is intentionally separate from canonical fixture setup. A
//! caller builds/publishes the returned source through the ordinary Git/build
//! path, imports it into a distinct instance, and then runs one policy-denied
//! ingress after the canonical recipe-42 positive control has completed.

use authz_postgres::PostgresMelangeAuthorizer;
use connectrpc::client::ClientConfig;
use forge_domain::RepositoryId;
use hephaestus_app::RunningHephaestus;
use rpc_proto::{
    connect::hephaestus::instance::v1::AgentInstanceServiceClient,
    messages::hephaestus::{
        common::v1::{OpaqueId, ParameterValue, RequestContext},
        instance::v1::BindSecretRequest,
        secret::v1::{DeliveryMode, DeliveryPhase},
    },
};
use secret_application::DeclareBrokeredHttpsRule;
use secret_domain::{AgentSecretBindingId, SecretCommandKey};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{error::Error, fmt::Write as _, sync::Arc, time::Duration};
use uuid::Uuid;

use super::cooking_builds::{self, CookingBuildContext, PreparedCookingInstance};

/// Result of one denied agent operation, retaining only opaque IDs and counts.
#[derive(Debug, Clone, Copy)]
pub struct AdversarialAgentProbe {
    /// The distinct adversarial mailbox event.
    pub event_id: Uuid,
    /// The run created for the event.
    pub run_id: Uuid,
    /// The fresh rule whose destination is deliberately incompatible.
    pub mismatched_rule_id: Uuid,
    /// Number of durable deny decisions for the run/rule.
    pub deny_decisions: i64,
    /// Number of durable substitution uses; must remain zero.
    pub substitution_uses: i64,
}

type RunAuditSummary = (Uuid, Uuid, String, i64, i64, i64, i64, i64);

/// The immutable rule and binding identities selected for an imported
/// instance. Callers use these IDs to register a broker adapter before the
/// daemon is restarted.
#[derive(Debug, Clone, Copy)]
pub struct BrokeredRuleSpec {
    /// Immutable broker rule identity claimed by the guest.
    pub rule_id: Uuid,
    /// Binding revision selected by the authenticated bind operation.
    pub binding_id: Uuid,
    /// Immutable instance revision owning the binding and rule.
    pub instance_revision_id: Uuid,
    /// Secret version pinned by that binding.
    pub secret_version_id: Uuid,
    /// Slot receiving the binding.
    pub slot: &'static str,
    /// Exact HTTPS origin authorized for the rule.
    pub destination: &'static str,
}

/// A prepared cooking instance together with both broker rule specs.
#[derive(Debug, Clone, Copy)]
pub struct PreparedBrokeredInstance {
    /// Imported instance and final immutable revision.
    pub instance: PreparedCookingInstance,
    /// Model binding/rule selected before import.
    pub model: BrokeredRuleSpec,
    /// Relay binding/rule selected before import.
    pub relay: BrokeredRuleSpec,
}

/// Stable rule identities registered by the shared upstream dispatcher.
#[derive(Debug, Clone, Copy)]
pub struct BrokeredRuleIds {
    /// Rule identity for the model destination.
    pub model: Uuid,
    /// Rule identity for the relay destination.
    pub relay: Uuid,
}

struct CanonicalBrokeredSecrets {
    model_import: Uuid,
    relay_import: Uuid,
    model_version: Uuid,
    relay_version: Uuid,
}

/// Imports and authorizes a separate adversarial instance.
///
/// Rule IDs are selected before `ImportAgent`, so the immutable imported
/// parameter document already refers to them. Each `BindSecret` revision
/// copies the previous active bindings to fresh IDs; after the second bind we
/// therefore read the final revision's binding IDs and declare the rules
/// through the authenticated service-level API on those exact bindings. No
/// later `ReviseInstance` is performed because that would clone the bindings
/// again without cloning their immutable rules.
pub async fn prepare_adversarial_instance(
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
pub async fn prepare_brokered_instance(
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

async fn canonical_brokered_secrets(
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
pub async fn prepare_brokered_instance_with_rule_ids(
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

async fn declare_final_rules(
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

/// Runs the canonical positive control, switches the installed gateway to the
/// adversarial mailbox through the existing Configure/CreateMailboxBinding
/// RPCs, and asserts a durable broker denial with no adapter-side effects. The
/// caller supplies the canonical recipe-42 audit baseline; the helper never
/// treats a process failure or timeout as a policy denial.
pub struct AdversarialAgentProbeInput<'a> {
    /// Shared production build/RPC context.
    pub context: &'a CookingBuildContext<'a>,
    /// Currently installed gateway declaration.
    pub gateway: cooking_builds::InstalledCookingGateway,
    /// Separate mailbox receiving the adversarial event.
    pub adversarial_instance: PreparedCookingInstance,
    /// Exact canonical runs whose adapter counts form the baseline.
    pub canonical_run_ids: [Uuid; 2],
    /// Model substitutions observed through the canonical positive control.
    pub baseline_model_calls: i64,
    /// Relay substitutions observed through the canonical positive control.
    pub baseline_relay_calls: i64,
    /// Fresh rule declared on the adversarial revision.
    pub adversarial_rule_id: Uuid,
    /// Existing inbound project import and exact selected version.
    pub inbound_import_id: Uuid,
    /// Existing inbound project import and exact selected version.
    pub inbound_version_id: Uuid,
    /// Nonsecret typed placeholder supplied to the gateway declaration.
    pub inbound_placeholder: &'a str,
    /// Actual selected inbound secret sent on the wire by the ingress client.
    pub inbound_wire_credential: &'a str,
    /// Public Caddy ingress endpoint.
    pub public_url: &'a str,
}

#[allow(clippy::too_many_lines)] // Keep the ordered ingress/audit acceptance proof readable.
pub async fn exercise_adversarial_agent_probe(
    input: AdversarialAgentProbeInput<'_>,
) -> Result<AdversarialAgentProbe, cooking_builds::BuildError> {
    let AdversarialAgentProbeInput {
        context,
        gateway,
        adversarial_instance,
        canonical_run_ids,
        baseline_model_calls,
        baseline_relay_calls,
        adversarial_rule_id,
        inbound_import_id,
        inbound_version_id,
        inbound_placeholder,
        inbound_wire_credential,
        public_url,
    } = input;
    let client = reqwest::Client::builder()
        .timeout(context.timeout)
        .build()?;

    let configured = cooking_builds::configure_cooking_gateway(
        context,
        gateway,
        cooking_builds::cooking_gateway_parameters(inbound_placeholder, 1001, 1002),
        inbound_import_id,
        inbound_version_id,
        adversarial_instance.mailbox_id,
    )
    .await?;
    let adversarial_id = unique_update_id();
    send_ingress(IngressRequest {
        pool: context.pool,
        client: &client,
        public_url,
        gateway_id: gateway.gateway_id,
        gateway_revision_id: configured.revision_id,
        mailbox_id: adversarial_instance.mailbox_id,
        update_id: adversarial_id,
        inbound_wire_credential,
        text: "adversarial-destination",
    })
    .await?;
    let (event_id, run_id) = wait_for_mailbox_run(
        context.pool,
        adversarial_instance.mailbox_id,
        &format!("telegram-update-{adversarial_id}"),
        false,
        context.timeout,
    )
    .await?;
    let audit_summary = run_audit_summary(context.pool, run_id).await?;
    if run_substitution_uses(&audit_summary) != 0 {
        eprintln!(
            "adversarial run audit summary event_id={event_id} run_id={run_id} rows={audit_summary:?}"
        );
        return Err(invalid("adversarial run reached credential substitution"));
    }
    let (model_after_denial, relay_after_denial): (i64, i64) = sqlx::query_as(
        "SELECT
             count(*) FILTER (WHERE audit.rule_id = $2),
             count(*) FILTER (WHERE audit.rule_id = $3)
           FROM brokered_secret_audit_events AS audit
           JOIN runs AS run ON run.id = audit.run_id
          WHERE run.id = ANY($1)
            AND audit.event_kind = 'substitution_use'",
    )
    .bind(canonical_run_ids.to_vec())
    .bind(super::cooking::MODEL_RULE)
    .bind(super::cooking::RELAY_RULE)
    .fetch_one(context.pool)
    .await?;
    if (model_after_denial, relay_after_denial) != (baseline_model_calls, baseline_relay_calls) {
        eprintln!(
            "adversarial run audit summary event_id={event_id} run_id={run_id} rows={audit_summary:?}"
        );
        return Err(invalid("denied broker operation changed adapter counters"));
    }
    let deny_decisions: i64 = sqlx::query_scalar(
        "SELECT count(*)\n           FROM brokered_secret_audit_events\n          WHERE run_id = $1 AND rule_id = $2\n            AND event_kind = 'authorization_decision'\n            AND decision = 'deny'",
    )
    .bind(run_id)
    .bind(adversarial_rule_id)
    .fetch_one(context.pool)
    .await?;
    let substitution_uses: i64 = sqlx::query_scalar(
        "SELECT count(*)\n           FROM brokered_secret_audit_events\n          WHERE run_id = $1 AND rule_id = $2\n            AND event_kind = 'substitution_use'",
    )
    .bind(run_id)
    .bind(adversarial_rule_id)
    .fetch_one(context.pool)
    .await?;
    if deny_decisions != 1 || substitution_uses != 0 {
        eprintln!(
            "adversarial run audit summary event_id={event_id} run_id={run_id} rows={audit_summary:?}"
        );
        return Err(invalid("broker denial audit evidence is incomplete"));
    }
    Ok(AdversarialAgentProbe {
        event_id,
        run_id,
        mismatched_rule_id: adversarial_rule_id,
        deny_decisions,
        substitution_uses,
    })
}

async fn run_audit_summary(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<RunAuditSummary>, sqlx::Error> {
    sqlx::query_as(
        "SELECT runtime_session_id, rule_id, event_kind, count(*)::bigint,
                count(*) FILTER (WHERE decision = 'allow')::bigint,
                count(*) FILTER (WHERE decision = 'deny')::bigint,
                count(*) FILTER (WHERE outcome = 'succeeded')::bigint,
                count(*) FILTER (WHERE outcome = 'failed')::bigint
           FROM brokered_secret_audit_events
          WHERE run_id = $1
          GROUP BY runtime_session_id, rule_id, event_kind
          ORDER BY runtime_session_id, rule_id, event_kind",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}

fn run_substitution_uses(rows: &[RunAuditSummary]) -> i64 {
    rows.iter()
        .filter(|(_, _, event_kind, _, _, _, _, _)| event_kind == "substitution_use")
        .map(|(_, _, _, events, _, _, _, _)| *events)
        .sum()
}

#[cfg(test)]
#[test]
fn run_substitution_uses_counts_every_rule_and_session() {
    let rows = vec![
        (
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            String::from("substitution_use"),
            1,
            0,
            0,
            1,
            0,
        ),
        (
            Uuid::from_u128(3),
            Uuid::from_u128(4),
            String::from("substitution_use"),
            2,
            0,
            0,
            2,
            0,
        ),
        (
            Uuid::from_u128(3),
            Uuid::from_u128(4),
            String::from("authorization_decision"),
            1,
            0,
            1,
            0,
            0,
        ),
    ];
    assert_eq!(run_substitution_uses(&rows), 3);
}

fn parameters(model_rule_id: Uuid, relay_rule_id: Uuid) -> Vec<ParameterValue> {
    [
        ("model_rule_id", model_rule_id),
        ("relay_rule_id", relay_rule_id),
    ]
    .into_iter()
    .map(|(name, value)| ParameterValue {
        name: name.to_owned(),
        value: Some(
            rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::StringValue(
                value.to_string(),
            ),
        ),
        ..Default::default()
    })
    .collect()
}

async fn bind_secret(
    context: &CookingBuildContext<'_>,
    input: BindSecretInput<'_>,
) -> Result<BoundSecret, cooking_builds::BuildError> {
    let BindSecretInput {
        instance_id,
        expected_revision_id,
        import_id,
        attachment_id,
        slot,
        destinations,
        operation,
    } = input;
    let audience = "/hephaestus.instance.v1.AgentInstanceService/BindSecret";
    let client = instance_client(context.running, context.identity.rpc_token, audience)?;
    let response = client
        .bind_secret(BindSecretRequest {
            context: mutation_context(operation).into(),
            instance_id: opaque(instance_id).into(),
            expected_revision_id: opaque(expected_revision_id).into(),
            import_id: opaque(import_id).into(),
            slot: slot.to_owned(),
            mode: DeliveryMode::Brokered.into(),
            phases: vec![DeliveryPhase::Normal.into()],
            attachment_ids: vec![opaque(attachment_id)],
            destinations: destinations
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let instance_revision_id = response
        .instance_revision_id
        .into_option()
        .ok_or_else(|| invalid("BindSecret returned no revision ID"))?
        .value
        .parse()?;
    Ok(BoundSecret {
        instance_revision_id,
    })
}

async fn declare_rule(
    service: &SecretService<LocalKeyProvider>,
    context: &CookingBuildContext<'_>,
    binding_id: Uuid,
    rule_id: Uuid,
    destination: &str,
    operation: &str,
) -> Result<(), cooking_builds::BuildError> {
    service
        .declare_brokered_https_rule(
            context.identity.actor,
            DeclareBrokeredHttpsRule {
                command_key: SecretCommandKey::derive(operation, &[rule_id.as_bytes()]),
                rule_id,
                binding_id: AgentSecretBindingId::from_uuid(binding_id),
                destination: destination.to_owned(),
                header: String::from("authorization"),
                header_prefix: Some(String::from("Bearer ")),
            },
        )
        .await
        .map_err(|error| Box::<dyn Error + Send + Sync>::from(error.to_string()))?;
    Ok(())
}

struct BoundSecret {
    instance_revision_id: Uuid,
}

struct BindSecretInput<'a> {
    instance_id: Uuid,
    expected_revision_id: Uuid,
    import_id: Uuid,
    attachment_id: Uuid,
    slot: &'a str,
    destinations: &'a [&'a str],
    operation: &'a str,
}

struct IngressRequest<'a> {
    pool: &'a PgPool,
    client: &'a reqwest::Client,
    public_url: &'a str,
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
    mailbox_id: Uuid,
    update_id: u64,
    inbound_wire_credential: &'a str,
    text: &'a str,
}

async fn send_ingress(request: IngressRequest<'_>) -> Result<(), cooking_builds::BuildError> {
    let IngressRequest {
        pool,
        client,
        public_url,
        gateway_id,
        gateway_revision_id,
        mailbox_id,
        update_id,
        inbound_wire_credential,
        text,
    } = request;
    let started_at = time::OffsetDateTime::now_utc();
    let response = client
        .post(format!("{public_url}/gateway/cooking/telegram"))
        .header("x-telegram-bot-api-secret-token", inbound_wire_credential)
        .json(&serde_json::json!({
            "update_id": update_id,
            "message": {"from": {"id": 1001}, "text": text}
        }))
        .send()
        .await?;
    let fingerprint = fingerprint_response(response).await?;
    if !(200..300).contains(&fingerprint.status) {
        let evidence = ingress_evidence(
            pool,
            gateway_id,
            gateway_revision_id,
            mailbox_id,
            update_id,
            started_at,
        )
        .await
        .unwrap_or_else(|_| String::from("durable_evidence=unavailable"));
        return Err(Box::<dyn Error + Send + Sync>::from(format!(
            "cooking ingress was not accepted by Caddy: update_id={update_id} status={} body_observed_bytes={} body_sha256_prefix={} body_observation_truncated={} gateway_id={gateway_id} revision_id={gateway_revision_id} mailbox_id={mailbox_id} {evidence}",
            fingerprint.status,
            fingerprint.body_observed_bytes,
            fingerprint.body_sha256_prefix,
            fingerprint.body_observation_truncated,
        )));
    }
    Ok(())
}

const MAX_INGRESS_BODY_HASH_BYTES: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct ResponseFingerprint {
    status: u16,
    body_observed_bytes: usize,
    body_sha256_prefix: String,
    body_observation_truncated: bool,
}

async fn fingerprint_response(
    mut response: reqwest::Response,
) -> Result<ResponseFingerprint, reqwest::Error> {
    let status = response.status().as_u16();
    let mut digest = Sha256::new();
    let mut body_observed_bytes = 0_usize;
    let mut hashed_len = 0_usize;
    while let Some(chunk) = response.chunk().await? {
        let remaining = MAX_INGRESS_BODY_HASH_BYTES.saturating_sub(hashed_len);
        if remaining == 0 {
            break;
        }
        let prefix = &chunk[..chunk.len().min(remaining)];
        digest.update(prefix);
        body_observed_bytes = body_observed_bytes.saturating_add(prefix.len());
        hashed_len += prefix.len();
        if prefix.len() < chunk.len() {
            break;
        }
    }
    Ok(ResponseFingerprint {
        status,
        body_observed_bytes,
        body_sha256_prefix: digest_hex(digest.finalize().as_slice()),
        body_observation_truncated: hashed_len >= MAX_INGRESS_BODY_HASH_BYTES,
    })
}

fn digest_hex(digest: &[u8]) -> String {
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Writing into a pre-sized String cannot fail.
        let _ = write!(output, "{byte:02x}");
    }
    output
}

async fn ingress_evidence(
    pool: &PgPool,
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
    mailbox_id: Uuid,
    update_id: u64,
    started_at: time::OffsetDateTime,
) -> Result<String, sqlx::Error> {
    let invocation_rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, outcome
           FROM gateway_invocations
          WHERE gateway_id = $1
            AND gateway_revision_id = $2
            AND accepted_at >= $3
          ORDER BY accepted_at DESC, id DESC
          LIMIT 8",
    )
    .bind(gateway_id)
    .bind(gateway_revision_id)
    .bind(started_at)
    .fetch_all(pool)
    .await?;
    let deduplication_key = format!("telegram-update-{update_id}");
    let publication: Option<(Option<Uuid>, String, Option<String>)> = sqlx::query_as(
        "SELECT event_id, outcome, denial_code
           FROM gateway_mailbox_publications
          WHERE gateway_revision_id = $1
            AND mailbox_id = $2
            AND deduplication_key = $3
          ORDER BY accepted_at DESC, id DESC
          LIMIT 1",
    )
    .bind(gateway_revision_id)
    .bind(mailbox_id)
    .bind(deduplication_key)
    .fetch_optional(pool)
    .await?;
    let gateway_state: Option<(Option<Uuid>, String)> = sqlx::query_as(
        "SELECT active_revision_id, lifecycle
           FROM gateways
          WHERE id = $1",
    )
    .bind(gateway_id)
    .fetch_optional(pool)
    .await?;
    let route: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT id, enabled
           FROM gateway_routes
          WHERE gateway_id = $1
            AND gateway_revision_id = $2
            AND path = '/cooking/telegram'
          LIMIT 1",
    )
    .bind(gateway_id)
    .bind(gateway_revision_id)
    .fetch_optional(pool)
    .await?;
    let invocations = invocation_rows
        .iter()
        .map(|(id, outcome)| format!("{id}:{outcome}"))
        .collect::<Vec<_>>()
        .join(",");
    let publication = publication.map_or_else(
        || String::from("none"),
        |(event_id, outcome, denial_code)| {
            format!(
                "event_id={}:{} denial_code={}",
                event_id.map_or_else(|| String::from("none"), |id| id.to_string()),
                outcome,
                denial_code.unwrap_or_else(|| String::from("none")),
            )
        },
    );
    let gateway_state = gateway_state.map_or_else(
        || String::from("none"),
        |(active_revision_id, lifecycle)| {
            format!(
                "active_revision_id={} lifecycle={lifecycle}",
                active_revision_id.map_or_else(|| String::from("none"), |id| id.to_string()),
            )
        },
    );
    let route = route.map_or_else(
        || String::from("none"),
        |(route_id, enabled)| format!("{route_id}:enabled={enabled}"),
    );
    Ok(format!(
        "invocations=[{invocations}] publication={publication} gateway_state=[{gateway_state}] route=[{route}]"
    ))
}

async fn wait_for_mailbox_run(
    pool: &PgPool,
    mailbox_id: Uuid,
    deduplication_key: &str,
    expect_success: bool,
    timeout: Duration,
) -> Result<(Uuid, Uuid), cooking_builds::BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let row: Option<(Uuid, Uuid, String, Option<String>)> = sqlx::query_as(
            "SELECT event.id, run.id, run.state, run.outcome\n               FROM mailbox_events event\n               JOIN mailbox_delivery_attempts attempt ON attempt.event_id = event.id\n               JOIN runs run ON run.id = attempt.run_id\n              WHERE event.mailbox_id = $1\n                AND event.deduplication_key = $2\n              ORDER BY attempt.attempt_number DESC\n              LIMIT 1",
        )
        .bind(mailbox_id)
        .bind(deduplication_key)
        .fetch_optional(pool)
        .await?;
        if let Some((event_id, run_id, state, outcome)) = row {
            if state == "cleaned_up"
                && outcome.as_deref()
                    == Some(if expect_success {
                        "succeeded"
                    } else {
                        "failed"
                    })
            {
                return Ok((event_id, run_id));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid(
                "cooking agent run did not reach its expected outcome",
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn instance_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<AgentInstanceServiceClient<connectrpc::client::HttpClient>, cooking_builds::BuildError>
{
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(AgentInstanceServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

fn mutation_context(operation: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: format!("cooking-adversarial-{operation}-{}", Uuid::new_v4()),
        ..Default::default()
    }
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

fn unique_update_id() -> u64 {
    let bytes = *Uuid::new_v4().as_bytes();
    bounded_update_id(u64::from_be_bytes(
        bytes[..8].try_into().expect("UUID prefix"),
    ))
}

fn bounded_update_id(candidate: u64) -> u64 {
    (candidate & MAX_SIGNED_UPDATE_ID).max(1)
}

const MAX_SIGNED_UPDATE_ID: u64 = 9_223_372_036_854_775_807;

#[cfg(test)]
#[test]
fn generated_adversarial_update_ids_fit_gateway_bounds_and_remain_unique() {
    assert_eq!(bounded_update_id(u64::MAX), MAX_SIGNED_UPDATE_ID);
    assert_eq!(bounded_update_id(0), 1);
    let ids: std::collections::HashSet<_> = (0..256).map(|_| unique_update_id()).collect();
    assert_eq!(ids.len(), 256);
    assert!(ids.iter().all(|id| *id <= MAX_SIGNED_UPDATE_ID));
}

fn invalid(message: &str) -> cooking_builds::BuildError {
    Box::<dyn Error + Send + Sync>::from(message.to_owned())
}
