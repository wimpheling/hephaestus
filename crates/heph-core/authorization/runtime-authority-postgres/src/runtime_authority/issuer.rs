use super::{
    HTTP_HANDLER_CONTRACT_V1, HTTP_SERVICE_HANDLER_CONTRACT_V1,
    gateway_helpers::{ensure_gateway_handler_contract, gateway_snapshot},
    gateway_repository::PgGatewayRuntimeSessionRepository,
};
use async_trait::async_trait;
use capability_domain::{RuntimeCredentialGeneration, RuntimeInvocation, RuntimeSessionId};
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest, RuntimeAuthorityError,
    RuntimeHandoffStore, RuntimeSessionIssuer, StoredRuntimeSession,
};
use sqlx::PgPool;

/// Gateway authority issuer built from the common session/handoff machinery.
pub struct PgGatewayRuntimeAuthorityIssuer<H> {
    issuer: RuntimeSessionIssuer<PgGatewayRuntimeSessionRepository, H>,
    pool: PgPool,
    authorization_model_version: String,
}

impl<H> PgGatewayRuntimeAuthorityIssuer<H>
where
    H: RuntimeHandoffStore,
{
    /// Constructs an issuer using the explicit authorization model version.
    #[must_use]
    pub fn new(pool: PgPool, handoff: H, authorization_model_version: impl Into<String>) -> Self {
        Self {
            issuer: RuntimeSessionIssuer::new(
                PgGatewayRuntimeSessionRepository::new(pool.clone()),
                handoff,
            ),
            pool,
            authorization_model_version: authorization_model_version.into(),
        }
    }
}

#[async_trait]
impl<H> GatewayRuntimeAuthorityIssuer for PgGatewayRuntimeAuthorityIssuer<H>
where
    H: RuntimeHandoffStore,
{
    async fn issue_gateway(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        ensure_gateway_handler_contract(&self.pool, request, HTTP_HANDLER_CONTRACT_V1).await?;
        let snapshot =
            gateway_snapshot(&self.pool, request, &self.authorization_model_version).await?;
        let identity = capability_domain::RuntimeSessionIdentity::new(
            RuntimeSessionId::from_uuid(request.invocation_id.as_uuid()),
            snapshot.principal(),
            RuntimeInvocation::Gateway(request.invocation_id),
            &snapshot,
            request.issued_at,
            request.expires_at,
        )
        .map_err(|_| RuntimeAuthorityError::Persistence)?;
        let issued = self
            .issuer
            .issue(&snapshot, &identity, None, request.issued_at)
            .await?;
        Ok(issued.session)
    }

    async fn issue_gateway_service(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        ensure_gateway_handler_contract(&self.pool, request, HTTP_SERVICE_HANDLER_CONTRACT_V1)
            .await?;
        let snapshot =
            gateway_snapshot(&self.pool, request, &self.authorization_model_version).await?;
        let identity = capability_domain::RuntimeSessionIdentity::new(
            RuntimeSessionId::from_uuid(request.invocation_id.as_uuid()),
            snapshot.principal(),
            RuntimeInvocation::Gateway(request.invocation_id),
            &snapshot,
            request.issued_at,
            request.expires_at,
        )
        .map_err(|_| RuntimeAuthorityError::Persistence)?;
        PgGatewayRuntimeSessionRepository::new(self.pool.clone())
            .create_host_mediated(&snapshot, &identity, RuntimeCredentialGeneration::INITIAL)
            .await
    }
}
