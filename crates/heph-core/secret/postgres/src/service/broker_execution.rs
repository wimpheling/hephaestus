use super::*;

impl<K: KeyProvider + Send + Sync> SecretRuntimeService<K> {
    /// Executes one semantic broker operation without returning plaintext to
    /// the guest.
    ///
    /// # Errors
    ///
    /// Fails closed for invalid bounded input, token theft, destination or
    /// capability mismatch, live authorization/lifecycle denial, encrypted
    /// value failure, adapter failure, or response overflow.
    #[tracing::instrument(
          skip_all,
          fields(
              run_id = %request.run_id,
              slot = request.slot.as_str(),
              destination = request.destination.as_str(),
              operation = request.operation.as_str()
          )
      )]
    #[allow(clippy::too_many_lines)] // Keep fail-closed broker phases together for audit review.
    pub async fn use_brokered<A: BrokerAdapter + ?Sized>(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        request: &BrokerRequest,
        adapter: &A,
    ) -> Result<BrokerResponse, SecretServiceError> {
        validate_broker_request(request)?;
        let session = match self.authenticate_session(credential, request.run_id).await {
            Ok(session) => session,
            Err(error @ SecretServiceError::RuntimeAuthenticationDenied) => {
                // A revocation can close the runtime session before the next
                // slot call arrives. The exact credential hash still lets us
                // identify that authenticated session for a value-free audit,
                // while the original authentication denial remains returned.
                self.record_pre_adapter_https_denial(
                    credential,
                    request,
                    &error,
                    PreAdapterDenialStage::SessionAuthentication,
                )
                .await?;
                return Err(error);
            }
            Err(error) => {
                Self::log_pre_adapter_https_denial(
                    request,
                    &error,
                    PreAdapterDenialStage::SessionAuthentication,
                );
                return Err(error);
            }
        };
        let lease = match self
            .authorize_runtime_lease(
                &session,
                &request.slot,
                DeliveryMode::Brokered,
                Permission::UseBrokered,
            )
            .await
        {
            Ok(lease) => lease,
            Err(error) => {
                self.record_pre_adapter_https_denial(
                    credential,
                    request,
                    &error,
                    PreAdapterDenialStage::LeaseAuthorization,
                )
                .await?;
                return Err(error);
            }
        };
        let (https_request, rule_id, verified_rule) = self
            .authorize_https_operation(&session, &lease, request)
            .await
            .inspect_err(|error| {
                Self::log_pre_adapter_https_denial(
                    request,
                    error,
                    PreAdapterDenialStage::RequestAuthorization,
                );
            })?;
        let (context, encrypted) = load_runtime_version(
            &self.resolver_pool,
            &session,
            &lease,
            DeliveryMode::Brokered,
            false,
            "broker_call_started",
        )
        .await
        .inspect_err(|error| {
            Self::log_pre_adapter_https_denial(
                request,
                error,
                PreAdapterDenialStage::VersionLoading,
            );
        })?;
        let value = self
            .encrypted_store
            .resolve(&context, &encrypted)
            .map_err(|error| {
                let service_error = SecretServiceError::Encryption(error);
                Self::log_pre_adapter_https_denial(
                    request,
                    &service_error,
                    PreAdapterDenialStage::Decryption,
                );
                service_error
            })?;
        let adapter_response = match verified_rule.as_ref() {
            Some(rule) => adapter.invoke_verified_https(&value, request, rule).await,
            None => {
                adapter
                    .invoke(
                        &value,
                        &request.destination,
                        &request.operation,
                        &request.body,
                    )
                    .await
            }
        };
        let response = match adapter_response {
            Ok(response) => response,
            Err(error) => {
                self.record_https_failure(&session, &lease, https_request, rule_id)
                    .await?;
                return Err(SecretServiceError::BrokerAdapter(error));
            }
        };
        if response.body.len() > 65_536 {
            self.record_https_failure(&session, &lease, https_request, rule_id)
                .await?;
            return Err(SecretServiceError::BrokerResponseTooLarge);
        }
        self.authorize_brokered_response(&session, &lease, request, https_request, rule_id)
            .await?;
        if let Some(request_id) = https_request {
            let succeeded = response.status == secret_application::BrokerStatus::Succeeded;
            record_https_operation(
                &self.resolver_pool,
                &session,
                &lease,
                request_id,
                rule_id,
                None,
                Some(if succeeded { "succeeded" } else { "failed" }),
                None,
            )
            .await?;
        }
        record_runtime_use(
            &self.resolver_pool,
            &session,
            &lease,
            "broker_call_completed",
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        Ok(response)
    }

    async fn record_https_failure(
        &self,
        session: &RuntimeSessionRow,
        lease: &RuntimeLeaseAuthorizationRow,
        request_id: Option<Uuid>,
        rule_id: Option<Uuid>,
    ) -> Result<(), SecretServiceError> {
        if let Some(request_id) = request_id {
            record_https_operation(
                &self.resolver_pool,
                session,
                lease,
                request_id,
                rule_id,
                None,
                Some("failed"),
                None,
            )
            .await?;
        }
        Ok(())
    }

    async fn record_pre_adapter_https_denial(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        request: &BrokerRequest,
        error: &SecretServiceError,
        stage: PreAdapterDenialStage,
    ) -> Result<(), SecretServiceError> {
        Self::log_pre_adapter_https_denial(request, error, stage);
        let reason_code = match error {
            // Session expiry and retirement are also represented by the
            // authentication denial; keep the audit reason generic rather
            // than claiming a specific revocation cause.
            SecretServiceError::Unavailable | SecretServiceError::RuntimeAuthenticationDenied => {
                "live_authorization_unavailable"
            }
            SecretServiceError::AuthorizationDenied => "live_authorization_denied",
            _ => return Ok(()),
        };
        if request.operation != "https_v1" {
            return Ok(());
        }
        let Some(rule_id) = broker_request_rule_id(request) else {
            return Ok(());
        };
        let origin = format!("https://{}", request.destination);
        let hash = credential.storage_hash();
        let Some(context) = sqlx::query_as::<_, BrokeredSecretDenialContextRow>(
            "SELECT session_id, run_id, instance_id, instance_revision_id,
                      attachment_id, phase, expires_at, lease_id,
                      secret_version_id, destinations
               FROM lookup_brokered_secret_denial_context($1, $2, $3, $4, $5)",
        )
        .bind(hash.as_slice())
        .bind(request.run_id.as_uuid())
        .bind(request.slot.as_str())
        .bind(rule_id)
        .bind(origin)
        .fetch_optional(&self.authorization_pool)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        else {
            // Without the exact retained lease and snapshot, a request rule
            // UUID is untrusted and must not receive an audit attribution.
            return Ok(());
        };
        let session = RuntimeSessionRow {
            session_id: context.session_id,
            run_id: context.run_id,
            instance_id: context.instance_id,
            instance_revision_id: context.instance_revision_id,
            attachment_id: context.attachment_id,
            phase: context.phase,
            expires_at: context.expires_at,
        };
        let lease = RuntimeLeaseAuthorizationRow {
            lease_id: context.lease_id,
            secret_version_id: context.secret_version_id,
            destinations: context.destinations,
        };
        record_https_operation(
            &self.resolver_pool,
            &session,
            &lease,
            Uuid::new_v4(),
            Some(rule_id),
            Some("deny"),
            None,
            Some(reason_code),
        )
        .await
    }

    /// Emits only a bounded, allowlisted classification for a denial that
    /// occurred before the upstream adapter was invoked. Error values can
    /// contain provider details, request data, or persistence diagnostics and
    /// therefore must never be attached to this log record.
    fn log_pre_adapter_https_denial(
        request: &BrokerRequest,
        error: &SecretServiceError,
        stage: PreAdapterDenialStage,
    ) {
        let denial_class = PreAdapterDenialClass::from_error(error);
        tracing::warn!(
            run_id = %request.run_id,
            slot = request.slot.as_str(),
            denial_stage = stage.as_str(),
            denial_class = denial_class.as_str(),
            "broker HTTPS request denied before upstream adapter"
        );
    }
}
