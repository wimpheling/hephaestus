use super::{
    Arc, GatewayReleaseArtifact, GatewayReleaseArtifactKind, GatewayReleaseMaterializer,
    GatewayRequestDispatcher, GatewayRuntimeLauncher, GatewayServiceArtifact,
    GatewayServiceArtifactKind, GatewayServiceBootRecoveryContext,
    GatewayServiceClaimResolutionStore, GatewayServiceExpiredClaimRecovery, GatewayServiceIdentity,
    GatewayServiceLogWriterConfig, GatewayServiceMaterializer, GatewayServiceSupervisorContext,
    LocalGatewayReleaseRuntime, LocalGatewayServiceIdentity, RunRuntimeArtifact,
    RunRuntimeArtifactKind, SocketAddr, UiOriginConfig, Uuid, VmInstance, VmMount, VmProvider,
    VmSpec, async_trait, ui_browser_content,
};

/// Runtime-owned dependencies for the optional shared-Caddy gateway edge.
pub struct GatewayEdgeRuntime {
    pub authority: super::PostgresGatewayEdgeAuthority,
    pub recovery_authority: super::PostgresGatewayEdgeAuthority,
    pub service_supervisor_context: Arc<GatewayServiceSupervisorContext>,
    pub service_claim_resolution: Arc<dyn GatewayServiceClaimResolutionStore>,
    pub service_expired_claim_recovery: Arc<dyn GatewayServiceExpiredClaimRecovery>,
    pub service_boot_context: GatewayServiceBootRecoveryContext,
    pub service_log_writer: GatewayServiceLogWriterConfig,
    pub provider: Arc<dyn gateway_edge::GatewayProvider>,
    pub dispatcher: Arc<dyn GatewayRequestDispatcher>,
    pub dispatcher_listen: SocketAddr,
    pub public_authority: String,
    pub ui_dispatcher: Option<Arc<dyn ui_browser_content::UiGatewayDispatcher>>,
    pub ui_origin: Option<UiOriginConfig>,
}

/// Narrow provider adapter used only after the gateway release resolver has
/// selected an exact immutable launch specification.
#[derive(Clone)]
pub struct ProviderGatewayRuntimeLauncher {
    pub provider: Arc<dyn VmProvider>,
}

#[async_trait]
impl GatewayRuntimeLauncher for ProviderGatewayRuntimeLauncher {
    async fn provision_gateway(
        &self,
        spec: VmSpec,
    ) -> Result<Arc<dyn VmInstance>, gateway_edge::GatewayEdgeError> {
        self.provider
            .provision(spec)
            .await
            .map_err(|_| gateway_edge::GatewayEdgeError::HandlerUnavailable)
    }
}

/// Composition adapter that gives gateway `PostgreSQL` authority the same
/// verified local release-artifact lifecycle used by ordinary runs.
#[derive(Clone)]
pub struct LocalGatewayReleaseMaterializer {
    pub runtime: LocalGatewayReleaseRuntime,
}

impl GatewayReleaseMaterializer for LocalGatewayReleaseMaterializer {
    fn prepare(
        &self,
        invocation_id: Uuid,
        artifacts: &[GatewayReleaseArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, gateway_edge::GatewayEdgeError> {
        let artifacts = artifacts
            .iter()
            .map(|artifact| RunRuntimeArtifact {
                path: artifact.path.clone(),
                kind: match artifact.kind {
                    GatewayReleaseArtifactKind::Executable => RunRuntimeArtifactKind::Executable,
                    GatewayReleaseArtifactKind::File => RunRuntimeArtifactKind::File,
                    GatewayReleaseArtifactKind::Manifest => RunRuntimeArtifactKind::Manifest,
                },
                mode: artifact.mode,
                content_hash: artifact.content_hash,
                size_bytes: artifact.size_bytes,
                storage_key: artifact.storage_key,
            })
            .collect::<Vec<_>>();
        self.runtime
            .prepare(invocation_id, &artifacts, parameters)
            .map_err(|_| gateway_edge::GatewayEdgeError::HandlerUnavailable)
    }

    fn destroy(&self, invocation_id: Uuid) -> Result<(), gateway_edge::GatewayEdgeError> {
        self.runtime
            .destroy(invocation_id)
            .map_err(|_| gateway_edge::GatewayEdgeError::HandlerUnavailable)
    }
}

impl GatewayServiceMaterializer for LocalGatewayReleaseMaterializer {
    fn prepare_service(
        &self,
        identity: GatewayServiceIdentity,
        artifacts: &[GatewayServiceArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, gateway_edge::GatewayEdgeError> {
        let identity = LocalGatewayServiceIdentity {
            instance_id: identity.instance_id,
            gateway_id: identity.gateway_id,
            revision_id: identity.revision_id,
        };
        let artifacts = artifacts
            .iter()
            .map(|artifact| RunRuntimeArtifact {
                path: artifact.path.clone(),
                kind: match artifact.kind {
                    GatewayServiceArtifactKind::Executable => RunRuntimeArtifactKind::Executable,
                    GatewayServiceArtifactKind::File => RunRuntimeArtifactKind::File,
                    GatewayServiceArtifactKind::Manifest => RunRuntimeArtifactKind::Manifest,
                },
                mode: artifact.mode,
                content_hash: artifact.content_hash,
                size_bytes: artifact.size_bytes,
                storage_key: artifact.storage_key,
            })
            .collect::<Vec<_>>();
        self.runtime
            .prepare_service(identity, &artifacts, parameters)
            .map_err(|_| gateway_edge::GatewayEdgeError::HandlerUnavailable)
    }

    fn destroy_service(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<(), gateway_edge::GatewayEdgeError> {
        self.runtime
            .destroy_service(LocalGatewayServiceIdentity {
                instance_id: identity.instance_id,
                gateway_id: identity.gateway_id,
                revision_id: identity.revision_id,
            })
            .map_err(|_| gateway_edge::GatewayEdgeError::HandlerUnavailable)
    }
}
