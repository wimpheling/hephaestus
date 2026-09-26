// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
/// Explicit fixture-only alias for a copied immutable broker rule.  The
/// production broker still receives the candidate rule ID; this adapter only
/// translates it to the pre-started TLS listener after the exact mapping was
/// registered by the update fixture.
pub(crate) struct RuleCopyAdapter {
    source_rule_id: uuid::Uuid,
    candidate_rule_id: uuid::Uuid,
    source: Arc<dyn secret_application::BrokerAdapter>,
}

#[async_trait::async_trait]
impl secret_application::BrokerAdapter for RuleCopyAdapter {
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<secret_application::BrokerResponse, secret_application::BrokerAdapterError> {
        let body = rewrite_rule_copy_request(body, self.source_rule_id, self.candidate_rule_id)?;
        self.source
            .invoke(credential, destination, operation, &body)
            .await
    }
}

/// Rewrites the two identifier-bearing fields that a copied rule presents to
/// the fixture's source adapter. The production broker has already checked
/// the candidate rule and substituted the credential; this narrow translation
/// preserves the source adapter's strict placeholder check without accepting
/// an arbitrary rule or header value.
pub(crate) fn rewrite_rule_copy_request(
    body: &[u8],
    source_rule_id: uuid::Uuid,
    candidate_rule_id: uuid::Uuid,
) -> Result<Vec<u8>, secret_application::BrokerAdapterError> {
    let mut request: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| secret_application::BrokerAdapterError::Rejected)?;
    let requested = request
        .get("rule_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.parse::<uuid::Uuid>().ok())
        .ok_or(secret_application::BrokerAdapterError::Rejected)?;
    if requested != candidate_rule_id {
        return Err(secret_application::BrokerAdapterError::Rejected);
    }
    let headers = request
        .get_mut("headers")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or(secret_application::BrokerAdapterError::Rejected)?;
    let candidate_placeholder = format!("Bearer heph-placeholder:v1:{candidate_rule_id}");
    let source_placeholder = format!("Bearer heph-placeholder:v1:{source_rule_id}");
    let mut replacements = 0_u8;
    for header in headers {
        if header.get("name").and_then(serde_json::Value::as_str) == Some("authorization")
            && header.get("value").and_then(serde_json::Value::as_str)
                == Some(candidate_placeholder.as_str())
        {
            header["value"] = serde_json::Value::String(source_placeholder.clone());
            replacements = replacements.saturating_add(1);
        }
    }
    if replacements != 1 {
        return Err(secret_application::BrokerAdapterError::Rejected);
    }
    request["rule_id"] = serde_json::Value::String(source_rule_id.to_string());
    serde_json::to_vec(&request).map_err(|_| secret_application::BrokerAdapterError::Rejected)
}

impl CookingAdapters {
    pub fn new(
        adapters: std::collections::HashMap<uuid::Uuid, Arc<dyn secret_application::BrokerAdapter>>,
    ) -> Self {
        Self(Arc::new(Mutex::new(adapters)))
    }

    /// Registers only the source/candidate pairs carried by one `CreateUpdate`
    /// request.  A missing source is a fixture construction error; no rule or
    /// origin fallback is permitted.
    pub fn register_rule_copies(&self, copies: &[(uuid::Uuid, uuid::Uuid)]) {
        let mut adapters = self.0.lock().expect("cooking adapter registry");
        for (source_rule_id, candidate_rule_id) in copies {
            let source = adapters
                .get(source_rule_id)
                .cloned()
                .expect("source broker rule adapter registered");
            assert_ne!(source_rule_id, candidate_rule_id);
            assert!(
                !adapters.contains_key(candidate_rule_id),
                "candidate broker rule ID must be allocated once"
            );
            adapters.insert(
                *candidate_rule_id,
                Arc::new(RuleCopyAdapter {
                    source_rule_id: *source_rule_id,
                    candidate_rule_id: *candidate_rule_id,
                    source,
                }),
            );
        }
    }

    pub fn snapshot(
        &self,
    ) -> std::collections::HashMap<uuid::Uuid, Arc<dyn secret_application::BrokerAdapter>> {
        self.0.lock().expect("cooking adapter registry").clone()
    }
}

#[async_trait::async_trait]
impl secret_application::BrokerAdapter for CookingAdapters {
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<secret_application::BrokerResponse, secret_application::BrokerAdapterError> {
        let request: BrokeredHttpsRequest = serde_json::from_slice(body)
            .map_err(|_| secret_application::BrokerAdapterError::Rejected)?;
        let adapter = self
            .0
            .lock()
            .expect("cooking adapter registry")
            .get(&request.rule_id)
            .cloned()
            .ok_or(secret_application::BrokerAdapterError::Rejected)?;
        adapter
            .invoke(credential, destination, operation, body)
            .await
    }
}

// The upstream fixture intentionally keeps TLS, request validation, and the
// bounded request ledger together so the test proves the whole provider edge.
#[allow(clippy::too_many_lines)]
pub(crate) async fn cooking_upstreams(rules: Vec<BrokeredSecretRule>) -> BrokeredTlsUpstream {
    cooking_upstreams_mode(rules, false).await
}

/// Starts the same TLS/ledger upstream factory for a transformed guest
/// release. Crash rules are keyed by immutable rule UUIDs, so canonical and
/// crash listeners can safely coexist on the same daemon adapter.
pub(crate) async fn cooking_crash_upstreams(rules: Vec<BrokeredSecretRule>) -> BrokeredTlsUpstream {
    cooking_upstreams_mode(rules, true).await
}
