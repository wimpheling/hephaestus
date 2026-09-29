use super::*;

impl<K: KeyProvider + Send + Sync> SecretRuntimeService<K> {
    pub(super) async fn authorize_brokered_response(
        &self,
        session: &RuntimeSessionRow,
        lease: &RuntimeLeaseAuthorizationRow,
        request: &BrokerRequest,
        request_id: Option<Uuid>,
        rule_id: Option<Uuid>,
    ) -> Result<(), SecretServiceError> {
        // A fresh live check prevents a response from being delivered after a
        // revocation that completed while the upstream operation was active.
        let live_authorization = self
            .authorize_runtime_lease(
                session,
                &request.slot,
                DeliveryMode::Brokered,
                Permission::UseBrokered,
            )
            .await;
        if let Err(
            error @ (SecretServiceError::Unavailable | SecretServiceError::AuthorizationDenied),
        ) = live_authorization
        {
            if let Some(request_id) = request_id {
                record_https_operation(
                    &self.resolver_pool,
                    session,
                    lease,
                    request_id,
                    // This ID was checked against this exact lease snapshot
                    // before the adapter ran; never use a fresh request value
                    // to attribute a denial to another lease.
                    rule_id,
                    Some("deny"),
                    None,
                    Some(if matches!(error, SecretServiceError::Unavailable) {
                        "live_authorization_unavailable"
                    } else {
                        "live_authorization_denied"
                    }),
                )
                .await?;
            }
            // Preserve the live rejection so callers cannot retry as though
            // the upstream operation had merely failed.
            return Err(error);
        }
        live_authorization?;
        if request.operation == "https_v1" {
            self.authorize_brokered_https_snapshot(session, lease, request, rule_id)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn authorize_https_operation(
        &self,
        session: &RuntimeSessionRow,
        lease: &RuntimeLeaseAuthorizationRow,
        request: &BrokerRequest,
    ) -> Result<
        (
            Option<Uuid>,
            Option<Uuid>,
            Option<VerifiedBrokeredHttpsRule>,
        ),
        SecretServiceError,
    > {
        let https_request = (request.operation == "https_v1").then(Uuid::new_v4);
        let rule_id = broker_request_rule_id(request);
        if !lease.destinations.is_empty()
            && !lease
                .destinations
                .iter()
                .any(|value| value == &request.destination)
        {
            if let Some(request_id) = https_request {
                record_https_operation(
                    &self.resolver_pool,
                    session,
                    lease,
                    request_id,
                    rule_id,
                    Some("deny"),
                    None,
                    None,
                )
                .await?;
            }
            return Err(SecretServiceError::BrokerRequestDenied);
        }
        if https_request.is_none() {
            return Ok((None, None, None));
        }
        let authorization = self
            .authorize_brokered_https_snapshot(session, lease, request, rule_id)
            .await;
        if let Some(request_id) = https_request {
            record_https_operation(
                &self.resolver_pool,
                session,
                lease,
                request_id,
                rule_id,
                Some(if authorization.is_ok() {
                    "allow"
                } else {
                    "deny"
                }),
                None,
                None,
            )
            .await?;
        }
        let verified_rule = authorization?;
        Ok((
            https_request,
            Some(verified_rule.rule_id).or(rule_id),
            Some(verified_rule),
        ))
    }
    /// Checks the immutable HTTPS rule snapshot for the generic broker
    /// operation. Legacy semantic broker operations deliberately retain their
    /// existing authority path; `https_v1` never does.
    async fn authorize_brokered_https_snapshot(
        &self,
        session: &RuntimeSessionRow,
        lease: &RuntimeLeaseAuthorizationRow,
        request: &BrokerRequest,
        rule_id: Option<Uuid>,
    ) -> Result<VerifiedBrokeredHttpsRule, SecretServiceError> {
        let rule_id = rule_id.ok_or(SecretServiceError::BrokerRequestDenied)?;
        let origin = format!("https://{}", request.destination);
        let found: Option<BrokeredHttpsRuleSnapshotRow> = sqlx::query_as(
            "SELECT snapshot.rule_id,
                    snapshot.destination_origin,
                    snapshot.header_name,
                    snapshot.header_prefix
               FROM brokered_secret_lease_snapshots AS snapshot
               JOIN secret_leases AS exact_lease ON exact_lease.id = snapshot.lease_id
               JOIN agent_secret_bindings AS binding ON binding.id = snapshot.binding_id
               JOIN secret_versions AS version ON version.id = snapshot.secret_version_id
               JOIN secrets AS secret ON secret.id = version.secret_id
               WHERE snapshot.lease_id = $1
                 AND snapshot.runtime_session_id = $2
                 AND snapshot.run_id = $3
                 AND snapshot.rule_id = $4
                 AND snapshot.destination_origin = $5
                 AND snapshot.binding_id = exact_lease.binding_id
                 AND snapshot.secret_version_id = exact_lease.secret_version_id
                 AND binding.status = 'active'
                 AND version.status = 'active'
                 AND secret.status = 'active'
                 -- Rotation changes the version selected for new leases. An
                 -- already-issued lease remains pinned until expiry or revoke.
                 AND exact_lease.status = 'active' AND exact_lease.expires_at > now()",
        )
        .bind(lease.lease_id)
        .bind(session.session_id)
        .bind(session.run_id)
        .bind(rule_id)
        .bind(origin)
        .fetch_optional(&self.authorization_pool)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        found
            .map(|row| VerifiedBrokeredHttpsRule {
                rule_id: row.rule_id,
                destination_origin: row.destination_origin,
                header_name: row.header_name,
                header_prefix: row.header_prefix,
            })
            .ok_or(SecretServiceError::BrokerRequestDenied)
    }
}
