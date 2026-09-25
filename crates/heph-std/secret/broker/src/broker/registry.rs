use super::{
    adapter::{destination_from_origin, destination_from_rule},
    common::{
        BrokerAdapter, BrokerAdapterError, BrokerRequest, BrokerResponse, BrokeredSecretRule,
        HashMap, IpAddr, SecretValue, VerifiedBrokeredHttpsRule, async_trait,
    },
    transport::ReqwestPinnedHttpsTransport,
    types::{
        BrokeredHttpsAdapter, BrokeredHttpsAdapterRegistry, BrokeredHttpsOrigin,
        BrokeredHttpsRequest, BrokeredHttpsUpstream, OriginPinnedHttpsAdapter,
    },
    validation::map_rule_error,
};

impl BrokeredHttpsAdapterRegistry {
    /// Builds a fail-closed registry from control-plane-pinned upstreams.
    ///
    /// # Errors
    ///
    /// Rejects duplicate rule IDs, invalid outbound rules, invalid DNS pins,
    /// and any upstream for which a certificate-verifying transport cannot be
    /// constructed.
    pub fn new(upstreams: Vec<BrokeredHttpsUpstream>) -> Result<Self, BrokerAdapterError> {
        Self::new_with_origin_catalog(upstreams, Vec::new())
    }

    /// Builds exact rule adapters and an optional origin transport catalog.
    ///
    /// Exact rule entries retain their legacy rule-ID behavior. Origin
    /// entries are a separate opt-in for rules declared after daemon startup;
    /// they never broaden an existing exact rule entry.
    ///
    /// # Errors
    ///
    /// Returns `Rejected` for duplicate entries, invalid origins, private or
    /// empty address pins, and invalid TLS transport configuration.
    pub fn new_with_origin_catalog(
        upstreams: Vec<BrokeredHttpsUpstream>,
        origins: Vec<BrokeredHttpsOrigin>,
    ) -> Result<Self, BrokerAdapterError> {
        let mut adapters = HashMap::with_capacity(upstreams.len());
        for upstream in upstreams {
            let rule = upstream.rule.normalized().map_err(map_rule_error)?;
            let destination = destination_from_rule(&rule)?;
            let transport = ReqwestPinnedHttpsTransport::new(&destination, &upstream.addresses)?;
            let id = rule.id.as_uuid();
            let adapter = BrokeredHttpsAdapter::new(rule, transport)?;
            if adapters.insert(id, adapter).is_some() {
                return Err(BrokerAdapterError::Rejected);
            }
        }
        let mut origin_adapters = HashMap::with_capacity(origins.len());
        for origin in origins {
            let destination = destination_from_origin(&origin.origin)?;
            let transport = ReqwestPinnedHttpsTransport::new(&destination, &origin.addresses)?;
            if origin_adapters
                .insert(
                    destination.clone(),
                    OriginPinnedHttpsAdapter {
                        destination,
                        transport,
                    },
                )
                .is_some()
            {
                return Err(BrokerAdapterError::Rejected);
            }
        }
        Ok(Self {
            adapters,
            origin_adapters,
        })
    }

    /// Builds one locally pinned, certificate-verifying adapter for an
    /// integration test.
    ///
    /// This is feature-gated so production configuration remains restricted
    /// to public, control-plane-pinned addresses. The supplied certificate is
    /// added as the only trust root; certificate and SNI validation still use
    /// the exact destination from the immutable rule.
    ///
    /// # Errors
    ///
    /// Returns an error when the rule is invalid, the local pin is malformed,
    /// the certificate is not valid PEM, or the TLS client cannot be built.
    #[cfg(feature = "test-fixtures")]
    pub fn test_only_local_trusted(
        rule: BrokeredSecretRule,
        port: u16,
        address: IpAddr,
        certificate_pem: &[u8],
    ) -> Result<Self, BrokerAdapterError> {
        let rule = rule.normalized().map_err(map_rule_error)?;
        let destination = destination_from_rule(&rule)?;
        let transport = ReqwestPinnedHttpsTransport::test_only_local_trusted_pin(
            &destination,
            port,
            address,
            certificate_pem,
        )?;
        let id = rule.id.as_uuid();
        let adapter = BrokeredHttpsAdapter::new(rule, transport)?;
        Ok(Self {
            adapters: HashMap::from([(id, adapter)]),
            origin_adapters: HashMap::new(),
        })
    }

    /// Builds a local CA-pinned origin catalog for integration tests.
    ///
    /// # Errors
    ///
    /// Returns `Rejected` when the origin, local pin, or certificate cannot be
    /// used to construct a trusted test transport.
    #[cfg(feature = "test-fixtures")]
    pub fn test_only_local_origin_catalog(
        origin: impl Into<String>,
        port: u16,
        address: IpAddr,
        certificate_pem: &[u8],
    ) -> Result<Self, BrokerAdapterError> {
        let destination = destination_from_origin(&origin.into())?;
        let transport = ReqwestPinnedHttpsTransport::test_only_local_trusted_pin(
            &destination,
            port,
            address,
            certificate_pem,
        )?;
        Ok(Self {
            adapters: HashMap::new(),
            origin_adapters: HashMap::from([(
                destination.clone(),
                OriginPinnedHttpsAdapter {
                    destination,
                    transport,
                },
            )]),
        })
    }
}

#[async_trait]
impl BrokerAdapter for BrokeredHttpsAdapterRegistry {
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        if operation != "https_v1" {
            return Err(BrokerAdapterError::Rejected);
        }
        let request: BrokeredHttpsRequest =
            serde_json::from_slice(body).map_err(|_| BrokerAdapterError::Rejected)?;
        let adapter = self
            .adapters
            .get(&request.rule_id)
            .ok_or(BrokerAdapterError::Rejected)?;
        adapter
            .invoke(credential, destination, operation, body)
            .await
    }

    async fn invoke_verified_https(
        &self,
        credential: &SecretValue,
        request: &BrokerRequest,
        rule: &VerifiedBrokeredHttpsRule,
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        if let Some(adapter) = self.adapters.get(&rule.rule_id) {
            return adapter
                .invoke(
                    credential,
                    &request.destination,
                    &request.operation,
                    &request.body,
                )
                .await;
        }
        let destination = destination_from_origin(&rule.destination_origin)?;
        let adapter = self
            .origin_adapters
            .get(&destination)
            .ok_or(BrokerAdapterError::Rejected)?;
        adapter.invoke_verified(credential, request, rule).await
    }
}
