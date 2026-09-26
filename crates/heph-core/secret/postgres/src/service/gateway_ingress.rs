use super::*;

impl<K: KeyProvider + Send + Sync> GatewayIngressSecretResolver<K> {
    /// Creates the resolver over the narrow worker pool and host KMS provider.
    #[must_use]
    pub const fn new(resolver_pool: PgPool, encrypted_store: EncryptedStore<K>) -> Self {
        Self {
            resolver_pool,
            encrypted_store,
        }
    }
}

#[async_trait]
impl<K: KeyProvider + Send + Sync> GatewayInboundSecretResolver
    for GatewayIngressSecretResolver<K>
{
    async fn rules_for_invocation(
        &self,
        invocation_id: Uuid,
        route_id: Uuid,
        gateway_revision_id: Uuid,
    ) -> Result<Vec<InboundGatewaySecretRule>, GatewayError> {
        let mut transaction = self
            .resolver_pool
            .begin()
            .await
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *transaction)
            .await
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
        let rows = sqlx::query_as::<_, GatewayEncryptedVersionRow>(
            "SELECT rule.header_name,
                    secret.id AS secret_id, version.id AS version_id, version.sequence,
                    secret.organization_id, secret.project_id,
                    version.algorithm, version.key_reference, version.data_nonce,
                    version.ciphertext, version.wrap_nonce, version.wrapped_data_key,
                    version.associated_data_hash, version.content_length
             FROM gateway_secret_leases AS lease
             JOIN gateway_runtime_authority_sessions AS session
               ON session.id = lease.runtime_session_id
              AND session.invocation_id = lease.invocation_id
             JOIN gateway_invocations AS invocation ON invocation.id = lease.invocation_id
             JOIN gateway_brokered_secret_rules AS rule ON rule.id = lease.rule_id
             JOIN gateway_secret_bindings AS binding ON binding.id = lease.binding_id
             JOIN secret_versions AS version ON version.id = lease.secret_version_id
             JOIN secrets AS secret ON secret.id = version.secret_id
             JOIN secret_imports AS imported ON imported.id = binding.import_id
             JOIN secret_grants AS granted ON granted.id = imported.grant_id
             WHERE lease.invocation_id = $1
               AND rule.gateway_route_id = $2
               AND invocation.gateway_revision_id = $3
               AND invocation.outcome = 'accepted'
               AND rule.gateway_revision_id = invocation.gateway_revision_id
               AND binding.gateway_revision_id = invocation.gateway_revision_id
               AND lease.secret_version_id = binding.secret_version_id
               AND lease.status = 'active' AND lease.expires_at > now()
               AND session.status IN ('pending_handoff', 'active') AND session.expires_at > now()
               AND binding.status = 'active' AND imported.status = 'active'
               AND granted.status = 'active' AND secret.status = 'active'
               AND version.status = 'active' AND version.revoked_at IS NULL
               AND version.purged_at IS NULL",
        )
        .bind(invocation_id)
        .bind(route_id)
        .bind(gateway_revision_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
        transaction
            .commit()
            .await
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
        rows.into_iter()
            .map(|row| {
                let header = HeaderName::from_bytes(row.header_name.as_bytes())
                    .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
                let (context, encrypted) = gateway_encrypted_version(row)?;
                let value = self
                    .encrypted_store
                    .resolve(&context, &encrypted)
                    .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
                let placeholder =
                    HeaderValue::from_str(&format!("heph-placeholder:v1:{}", context.version_id))
                        .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
                InboundGatewaySecretRule::new(header, value.expose().to_vec(), placeholder)
            })
            .collect()
    }
}
