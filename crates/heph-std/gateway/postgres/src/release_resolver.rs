//! PostgreSQL-backed gateway release resolution.

use super::{
    GatewayEdgeError, GatewayNetwork, GatewayReleaseMaterializer, GatewayRouteBinding,
    GatewayRuntimeContract, gateway_release_artifacts,
};
use async_trait::async_trait;
use gateway_domain::GatewayReleaseResolver;
use runtime_authority::RuntimeHandoffStore;
use sqlx::PgPool;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, RuntimeAuthorityBootstrap, VmId, VmResources, VmSpec,
};

/// Production resolver for the exact released agent selected by a gateway revision.
///
/// It accepts only an already-recorded invocation ID; public request data cannot
/// influence image, command, resource, or bootstrap selection.
pub struct PostgresGatewayReleaseResolver {
    pool: PgPool,
    root_images: BTreeMap<String, RootFilesystem>,
    handoff: Arc<dyn RuntimeHandoffStore>,
    release_materializer: Option<Arc<dyn GatewayReleaseMaterializer>>,
}

impl PostgresGatewayReleaseResolver {
    /// Creates a resolver over operator-materialized immutable root filesystems
    /// and the same host-only handoff store used to issue gateway sessions.
    #[must_use]
    pub fn new(
        pool: PgPool,
        root_images: BTreeMap<String, RootFilesystem>,
        handoff: Arc<dyn RuntimeHandoffStore>,
    ) -> Self {
        Self {
            pool,
            root_images,
            handoff,
            release_materializer: None,
        }
    }

    /// Attaches the host-owned release-tree lifecycle required for a real VM
    /// launch. A resolver without this explicit boundary fails closed rather
    /// than executing a path supplied by the base image.
    #[must_use]
    pub fn with_release_materializer(
        mut self,
        release_materializer: Arc<dyn GatewayReleaseMaterializer>,
    ) -> Self {
        self.release_materializer = Some(release_materializer);
        self
    }

    /// Cleans up the invocation's materialized release tree after the VM has
    /// been destroyed.
    ///
    /// # Errors
    ///
    /// Returns an unavailable edge error when no configured materializer can
    /// safely remove the invocation's release tree.
    pub fn destroy_materialized_release(
        &self,
        invocation_id: Uuid,
    ) -> Result<(), GatewayEdgeError> {
        self.release_materializer
            .as_ref()
            .ok_or(GatewayEdgeError::HandlerUnavailable)?
            .destroy(invocation_id)
    }
}

#[async_trait]
impl GatewayReleaseResolver for PostgresGatewayReleaseResolver {
    // The complete authority query, artifact validation, and fail-closed VM
    // specification are deliberately adjacent for one auditable launch path.
    #[allow(clippy::too_many_lines)]
    async fn resolve_launch(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
    ) -> Result<VmSpec, GatewayEdgeError> {
        let row = sqlx::query_as::<_, GatewayLaunchRow>(
            "SELECT agent.runtime_contract, session.issuance_generation, revision.release_id, revision.parameters
             FROM gateway_invocations AS invocation
             JOIN gateway_revisions AS revision
               ON revision.id = invocation.gateway_revision_id
              AND revision.gateway_id = invocation.gateway_id
             JOIN release_agents AS agent
               ON agent.id = revision.release_agent_id
              AND agent.release_id = revision.release_id
              AND agent.agent_key = revision.release_agent_key
             JOIN releases AS release
               ON release.id = revision.release_id
              AND release.repository_id = revision.repository_id
             JOIN gateway_runtime_authority_sessions AS session
               ON session.invocation_id = invocation.id
              AND session.gateway_id = invocation.gateway_id
              AND session.gateway_revision_id = invocation.gateway_revision_id
             WHERE invocation.id = $1
               AND invocation.gateway_route_id = $2
               AND invocation.gateway_revision_id = $3
               AND invocation.outcome = 'accepted'
               AND revision.handler_contract = 'http.v1'
               AND release.state = 'published'
               AND session.admission_mode = 'guest_handoff'
               AND session.status = 'pending_handoff'
               AND session.expires_at > now()",
        )
        .bind(invocation_id)
        .bind(route.route_id)
        .bind(route.gateway_revision_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?
        .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let contract: GatewayRuntimeContract = serde_json::from_value(row.runtime_contract)
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        // MVP-03 gateway VMs are stateless one-request handlers. They cannot
        // silently acquire a volume or general network listener merely because
        // the source agent also supports those modes elsewhere.
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
            .release_materializer
            .as_ref()
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let artifacts = gateway_release_artifacts(&self.pool, row.release_id).await?;
        let mounts = materializer.prepare(invocation_id, &artifacts, &row.parameters)?;
        let generation = u64::try_from(row.issuance_generation)
            .ok()
            .and_then(|value| capability_domain::RuntimeCredentialGeneration::new(value).ok())
            .ok_or(GatewayEdgeError::Unavailable)?;
        let session_id = capability_domain::RuntimeSessionId::from_uuid(invocation_id);
        let credential = self
            .handoff
            .open(session_id, generation, OffsetDateTime::now_utc())
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
        Ok(VmSpec {
            id: VmId(format!("gateway-{invocation_id}")),
            root,
            disks: Vec::new(),
            mounts,
            resources: VmResources {
                vcpus: contract.policy_ceiling.vcpus,
                memory_mib: contract.policy_ceiling.memory_mib,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: format!("/release/{}", contract.command),
                args: contract.arguments,
                env: BTreeMap::new(),
                working_dir: Some(PathBuf::from(format!(
                    "/release/{}",
                    contract.working_directory
                ))),
            },
            runtime_authority: Some(RuntimeAuthorityBootstrap::new(
                invocation_id,
                generation.get(),
                *credential.expose(),
            )),
            private_http_service: None,
            runtime_git_bridge: None,
            labels: BTreeMap::from([
                (String::from("hephaestus.kind"), String::from("gateway")),
                (
                    String::from("hephaestus.gateway.handler-contract"),
                    String::from("http.v1"),
                ),
                (
                    String::from("hephaestus.gateway-invocation"),
                    invocation_id.to_string(),
                ),
                (
                    String::from("hephaestus.gateway-route"),
                    route.route_id.to_string(),
                ),
                (
                    String::from("hephaestus.gateway-revision"),
                    route.gateway_revision_id.to_string(),
                ),
            ]),
        })
    }

    async fn acknowledge_runtime_authority(
        &self,
        invocation_id: Uuid,
        session_id: Uuid,
        generation: u64,
    ) -> Result<(), GatewayEdgeError> {
        let generation = i64::try_from(generation).map_err(|_| GatewayEdgeError::Unavailable)?;
        let acknowledged = sqlx::query_scalar::<_, bool>(
            "UPDATE gateway_runtime_authority_sessions
                SET status = 'active', acknowledged_at = now(), updated_at = now()
              WHERE id = $1
                AND invocation_id = $2
                AND issuance_generation = $3
                AND status = 'pending_handoff'
                AND issued_at <= now()
                AND expires_at > now()
              RETURNING TRUE",
        )
        .bind(session_id)
        .bind(invocation_id)
        .bind(generation)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        if acknowledged == Some(true) {
            Ok(())
        } else {
            Err(GatewayEdgeError::HandlerUnavailable)
        }
    }

    async fn cleanup_launch(&self, invocation_id: Uuid) -> Result<(), GatewayEdgeError> {
        self.destroy_materialized_release(invocation_id)
    }
}

#[derive(sqlx::FromRow)]
struct GatewayLaunchRow {
    runtime_contract: serde_json::Value,
    issuance_generation: i64,
    release_id: Uuid,
    parameters: serde_json::Value,
}
