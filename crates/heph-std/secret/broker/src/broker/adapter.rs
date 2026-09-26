use super::{
    common::{
        BrokerAdapter, BrokerAdapterError, BrokerRequest, BrokerResponse, BrokeredSecretRule,
        InjectionDirection, MAX_ADAPTER_BODY_BYTES, SecretValue, VerifiedBrokeredHttpsRule,
        async_trait,
    },
    types::{
        BrokeredHttpsAdapter, BrokeredHttpsRequest, OriginPinnedHttpsAdapter, PinnedHttpsTransport,
        UpstreamHttpsRequest,
    },
    validation::{
        map_rule_error, valid_dns_destination, validate_and_substitute_headers,
        validate_and_substitute_verified_headers, validate_https_path,
    },
};

impl<T> BrokeredHttpsAdapter<T> {
    /// Creates an adapter from one immutable outbound authority rule.
    ///
    /// # Errors
    ///
    /// Rejects inbound rules, broad destinations, and non-default HTTPS ports.
    /// The legacy runtime secret ceiling is a DNS host, not a URL, so explicit
    /// ports are deliberately deferred until that authority representation is
    /// extended consistently.
    pub fn new(rule: BrokeredSecretRule, transport: T) -> Result<Self, BrokerAdapterError> {
        let rule = rule.normalized().map_err(map_rule_error)?;
        if rule.location.direction() != InjectionDirection::Outbound {
            return Err(BrokerAdapterError::Rejected);
        }
        let destination = destination_from_rule(&rule)?;
        Ok(Self {
            destination,
            rule,
            transport,
        })
    }
}

pub(super) fn destination_from_rule(
    rule: &BrokeredSecretRule,
) -> Result<String, BrokerAdapterError> {
    let origin = rule
        .destination
        .as_ref()
        .ok_or(BrokerAdapterError::Rejected)?
        .as_str();
    destination_from_origin(origin)
}

pub(super) fn destination_from_origin(origin: &str) -> Result<String, BrokerAdapterError> {
    let destination = origin
        .strip_prefix("https://")
        .ok_or(BrokerAdapterError::Rejected)?;
    // The secret runtime's destination authority deliberately uses a DNS host
    // rather than an authority-with-port, so non-default ports cannot be
    // represented consistently yet.
    if destination.contains(':') || !valid_dns_destination(destination) {
        return Err(BrokerAdapterError::Rejected);
    }
    Ok(destination.to_owned())
}

impl OriginPinnedHttpsAdapter {
    pub(super) async fn invoke_verified(
        &self,
        credential: &SecretValue,
        request: &BrokerRequest,
        rule: &VerifiedBrokeredHttpsRule,
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        invoke_dynamic_pinned_request(
            &self.transport,
            &self.destination,
            credential,
            request,
            rule,
        )
        .await
    }
}

pub(super) async fn invoke_dynamic_pinned_request<T: PinnedHttpsTransport>(
    transport: &T,
    expected_destination: &str,
    credential: &SecretValue,
    request: &BrokerRequest,
    rule: &VerifiedBrokeredHttpsRule,
) -> Result<BrokerResponse, BrokerAdapterError> {
    if request.destination != expected_destination
        || request.operation != "https_v1"
        || request.body.len() > MAX_ADAPTER_BODY_BYTES
        || rule.destination_origin != format!("https://{expected_destination}")
    {
        return Err(BrokerAdapterError::Rejected);
    }
    let request: BrokeredHttpsRequest =
        serde_json::from_slice(&request.body).map_err(|_| BrokerAdapterError::Rejected)?;
    if request.rule_id != rule.rule_id {
        return Err(BrokerAdapterError::Rejected);
    }
    let headers = validate_and_substitute_verified_headers(&request, rule, credential.expose())?;
    let response = transport
        .send(UpstreamHttpsRequest {
            method: request.method,
            destination: expected_destination.to_owned(),
            path_and_query: validate_https_path(&request.path_and_query)?,
            headers,
            body: request.body,
        })
        .await?;
    if response
        .body
        .windows(credential.expose().len())
        .any(|window| window == credential.expose())
    {
        return Err(BrokerAdapterError::Rejected);
    }
    Ok(response)
}

#[async_trait]
impl<T> BrokerAdapter for BrokeredHttpsAdapter<T>
where
    T: PinnedHttpsTransport,
{
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        if destination != self.destination
            || operation != "https_v1"
            || body.len() > MAX_ADAPTER_BODY_BYTES
        {
            return Err(BrokerAdapterError::Rejected);
        }
        let request: BrokeredHttpsRequest =
            serde_json::from_slice(body).map_err(|_| BrokerAdapterError::Rejected)?;
        if request.rule_id != self.rule.id.as_uuid() {
            return Err(BrokerAdapterError::Rejected);
        }
        let headers = validate_and_substitute_headers(&request, &self.rule, credential.expose())?;
        let response = self
            .transport
            .send(UpstreamHttpsRequest {
                method: request.method,
                destination: self.destination.clone(),
                path_and_query: validate_https_path(&request.path_and_query)?,
                headers,
                body: request.body,
            })
            .await?;
        // An allowed upstream is still untrusted with respect to the guest:
        // it could reflect an authorization header in its body.  Never turn a
        // host-only substitution into plaintext delivery through a response.
        if response
            .body
            .windows(credential.expose().len())
            .any(|window| window == credential.expose())
        {
            return Err(BrokerAdapterError::Rejected);
        }
        Ok(response)
    }
}
