//! PostgreSQL-backed gateway service launch resolution.

use super::{
    GatewayEdgeError, GatewayNetwork, GatewayReleaseArtifactKind, GatewayRuntimeContract,
    gateway_release_artifacts, service_vm_spec,
};
use async_trait::async_trait;
use gateway_domain::{
    GatewayServiceArtifact, GatewayServiceArtifactKind, GatewayServiceConfig,
    GatewayServiceIdentity, GatewayServiceLaunch, GatewayServiceLaunchRequest,
    GatewayServiceLaunchResolver, GatewayServiceMaterializer, ServiceLogCaptureMode,
    ServiceProbePath,
};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;
use vm_trait::RootFilesystem;

/// Production resolver for one immutable published long-lived gateway service.
///
/// Unlike the stateless resolver, this port takes only the host-owned launch
/// identity. It does not require or create an invocation, runtime session, or
/// request bearer.
pub struct PostgresGatewayServiceLaunchResolver {
    pool: PgPool,
    root_images: BTreeMap<String, RootFilesystem>,
    service_materializer: Option<Arc<dyn GatewayServiceMaterializer>>,
}

impl PostgresGatewayServiceLaunchResolver {
    /// Creates a service resolver over the immutable gateway database and root
    /// image catalog.
    #[must_use]
    pub fn new(pool: PgPool, root_images: BTreeMap<String, RootFilesystem>) -> Self {
        Self {
            pool,
            root_images,
            service_materializer: None,
        }
    }

    /// Attaches the host-owned persistent-service materializer.
    #[must_use]
    pub fn with_service_materializer(
        mut self,
        service_materializer: Arc<dyn GatewayServiceMaterializer>,
    ) -> Self {
        self.service_materializer = Some(service_materializer);
        self
    }
}

#[async_trait]
impl GatewayServiceLaunchResolver for PostgresGatewayServiceLaunchResolver {
    // Keep the exact query, validation, materialization, and postcondition
    // cleanup together as one auditable immutable launch boundary.
    #[allow(clippy::too_many_lines)]
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        let identity = request.identity;
        if identity.instance_id.is_nil()
            || identity.gateway_id.is_nil()
            || identity.revision_id.is_nil()
        {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        let row = sqlx::query_as::<_, GatewayServiceLaunchRow>(
            "SELECT revision.gateway_id, revision.id AS revision_id,
                    revision.release_id, revision.handler_contract,
                    revision.service_loopback_port,
                    revision.service_readiness_path, revision.service_health_path,
                    revision.service_log_capture_mode,
                    revision.parameters, agent.runtime_contract
               FROM gateway_revisions AS revision
               JOIN release_agents AS agent
                 ON agent.id = revision.release_agent_id
                AND agent.release_id = revision.release_id
                AND agent.agent_key = revision.release_agent_key
               JOIN releases AS release
                 ON release.id = revision.release_id
                AND release.repository_id = revision.repository_id
              WHERE revision.gateway_id = $1
                AND revision.id = $2
                AND revision.handler_contract = 'http.service.v1'
                AND release.state = 'published'",
        )
        .bind(identity.gateway_id)
        .bind(identity.revision_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?
        .ok_or(GatewayEdgeError::HandlerUnavailable)?;

        let release_id = row.release_id.ok_or(GatewayEdgeError::HandlerUnavailable)?;
        if row.gateway_id != identity.gateway_id || row.revision_id != identity.revision_id {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        if row.handler_contract != "http.service.v1" {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        let loopback_port = row
            .service_loopback_port
            .and_then(|value| u16::try_from(value).ok())
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let readiness_path = ServiceProbePath::parse(
            row.service_readiness_path
                .ok_or(GatewayEdgeError::HandlerUnavailable)?,
        )
        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
        let health_path = ServiceProbePath::parse(
            row.service_health_path
                .ok_or(GatewayEdgeError::HandlerUnavailable)?,
        )
        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
        let log_capture_mode = ServiceLogCaptureMode::from_name(&row.service_log_capture_mode)
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let service = GatewayServiceConfig::new(loopback_port, readiness_path, health_path)
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
            .with_log_capture_mode(log_capture_mode);
        let contract: GatewayRuntimeContract = serde_json::from_value(row.runtime_contract)
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
        if contract.requires_state
            || !matches!(contract.policy_ceiling.network, GatewayNetwork::Disabled)
        {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        let root = self
            .root_images
            .get(&contract.image_reference)
            .cloned()
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let materializer = self
            .service_materializer
            .as_ref()
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let artifacts = gateway_release_artifacts(&self.pool, release_id)
            .await?
            .into_iter()
            .map(|artifact| GatewayServiceArtifact {
                path: artifact.path,
                kind: match artifact.kind {
                    GatewayReleaseArtifactKind::Executable => {
                        GatewayServiceArtifactKind::Executable
                    }
                    GatewayReleaseArtifactKind::File => GatewayServiceArtifactKind::File,
                    GatewayReleaseArtifactKind::Manifest => GatewayServiceArtifactKind::Manifest,
                },
                mode: artifact.mode,
                content_hash: artifact.content_hash,
                size_bytes: artifact.size_bytes,
                storage_key: artifact.storage_key,
            })
            .collect::<Vec<_>>();
        let mounts = materializer.prepare_service(identity, &artifacts, &row.parameters)?;
        let spec = match service_vm_spec(identity, &service, contract, root, mounts) {
            Ok(spec) => spec,
            Err(error) => {
                let _ = materializer.destroy_service(identity);
                return Err(error);
            }
        };
        Ok(GatewayServiceLaunch {
            identity,
            service,
            spec,
        })
    }

    async fn cleanup_service_launch(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        self.service_materializer
            .as_ref()
            .ok_or(GatewayEdgeError::HandlerUnavailable)?
            .destroy_service(identity)
    }
}

#[derive(sqlx::FromRow)]
struct GatewayServiceLaunchRow {
    gateway_id: Uuid,
    revision_id: Uuid,
    release_id: Option<Uuid>,
    handler_contract: String,
    service_loopback_port: Option<i32>,
    service_readiness_path: Option<String>,
    service_health_path: Option<String>,
    service_log_capture_mode: String,
    parameters: serde_json::Value,
    runtime_contract: serde_json::Value,
}
