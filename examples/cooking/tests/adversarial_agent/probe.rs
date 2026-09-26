// Reuse the adversarial facade imports across the focused phases.
#[allow(unused_imports)]
use super::*;
/// Runs the canonical positive control, switches the installed gateway to the
/// adversarial mailbox through the existing Configure/CreateMailboxBinding
/// RPCs, and asserts a durable broker denial with no adapter-side effects. The
/// caller supplies the canonical recipe-42 audit baseline; the helper never
/// treats a process failure or timeout as a policy denial.
pub(crate) struct AdversarialAgentProbeInput<'a> {
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
pub(crate) async fn exercise_adversarial_agent_probe(
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
    let client = super::super::cooking::caddy_gateway_client_with_timeout(context.timeout);

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
    .bind(super::super::cooking::MODEL_RULE)
    .bind(super::super::cooking::RELAY_RULE)
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

pub(crate) async fn run_audit_summary(
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

pub(crate) fn run_substitution_uses(rows: &[RunAuditSummary]) -> i64 {
    rows.iter()
        .filter(|(_, _, event_kind, _, _, _, _, _)| event_kind == "substitution_use")
        .map(|(_, _, _, events, _, _, _, _)| *events)
        .sum()
}

#[cfg(test)]
#[test]
pub(crate) fn run_substitution_uses_counts_every_rule_and_session() {
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
