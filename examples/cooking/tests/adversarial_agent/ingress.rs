// Reuse the adversarial facade imports across the focused phases.
#[allow(unused_imports)]
use super::*;
pub(crate) fn parameters(model_rule_id: Uuid, relay_rule_id: Uuid) -> Vec<ParameterValue> {
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

pub(crate) async fn bind_secret(
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

pub(crate) async fn declare_rule(
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

pub(crate) struct BoundSecret {
    pub(crate) instance_revision_id: Uuid,
}

pub(crate) struct BindSecretInput<'a> {
    pub(crate) instance_id: Uuid,
    pub(crate) expected_revision_id: Uuid,
    pub(crate) import_id: Uuid,
    pub(crate) attachment_id: Uuid,
    pub(crate) slot: &'a str,
    pub(crate) destinations: &'a [&'a str],
    pub(crate) operation: &'a str,
}

pub(crate) struct IngressRequest<'a> {
    pub(crate) pool: &'a PgPool,
    pub(crate) client: &'a reqwest::Client,
    pub(crate) public_url: &'a str,
    pub(crate) gateway_id: Uuid,
    pub(crate) gateway_revision_id: Uuid,
    pub(crate) mailbox_id: Uuid,
    pub(crate) update_id: u64,
    pub(crate) inbound_wire_credential: &'a str,
    pub(crate) text: &'a str,
}

pub(crate) async fn send_ingress(
    request: IngressRequest<'_>,
) -> Result<(), cooking_builds::BuildError> {
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
pub(crate) struct ResponseFingerprint {
    status: u16,
    body_observed_bytes: usize,
    body_sha256_prefix: String,
    body_observation_truncated: bool,
}

pub(crate) async fn fingerprint_response(
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

pub(crate) fn digest_hex(digest: &[u8]) -> String {
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Writing into a pre-sized String cannot fail.
        let _ = write!(output, "{byte:02x}");
    }
    output
}

pub(crate) async fn ingress_evidence(
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
