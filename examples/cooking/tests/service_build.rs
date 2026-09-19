//! Release publication helper for the persistent `cooking-service` fixture.
//!
//! The published-service acceptance branch delegates source copying, Git push,
//! build observation, draft versioning, and release publication to the
//! existing cooking build helper rather than reproducing those operations.

use super::cooking_builds::{BuildError, CookingBuildContext, PublishedCookingRepository};
use rpc_proto::messages::hephaestus::gateway::v1::{
    ConfigureGatewayRequest, GetGatewayRequest, InstallReleaseGatewaysRequest,
    ListProjectGatewaysRequest,
};
use uuid::Uuid;

/// Gateway identity returned after the published service is installed and
/// configured. The revision is the new immutable revision returned by
/// `ConfigureGateway` and is the identity used by readiness assertions.
#[derive(Debug, Clone, Copy)]
pub struct ConfiguredCookingService {
    /// Gateway metadata identifier returned by the install query.
    pub gateway_id: Uuid,
    /// Configured immutable service revision identifier.
    pub revision_id: Uuid,
}

/// Builds and publishes the immutable `cooking-service` release.
///
/// The returned repository record contains the published release and agent
/// identities required by the later `InstallReleaseGateways` and
/// `ConfigureGateway` calls. The service declaration itself remains owned by
/// `heph.gateways.toml` in the published source.
pub async fn build_and_publish_cooking_service(
    context: &CookingBuildContext<'_>,
) -> Result<PublishedCookingRepository, BuildError> {
    // `build_one` is the existing production Git/build/release path. This
    // helper keeps the published-service proof on that same boundary.
    super::cooking_builds::build_one(
        context,
        "cooking-service",
        context.source_root.join("cooking-service"),
        "Cooking service",
        false,
    )
    .await
}

/// Installs and configures a published service declaration through the
/// production gateway RPCs.
///
/// The cooking service declares no typed parameters, secret slots, or mailbox
/// slots, so configuration deliberately submits empty selections. Readiness
/// and Caddy assertions should use the returned gateway and configured
/// revision IDs.
pub async fn install_and_configure_cooking_service(
    context: &CookingBuildContext<'_>,
    published: &PublishedCookingRepository,
) -> Result<ConfiguredCookingService, BuildError> {
    let client = super::cooking_builds::rpc_gateway_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.gateway.v1.GatewayService/InstallReleaseGateways",
    )?;
    client
        .install_release_gateways(InstallReleaseGatewaysRequest {
            context: super::cooking_builds::mutation_context("install-cooking-service").into(),
            release_id: super::cooking_builds::opaque(published.release_id).into(),
            ..Default::default()
        })
        .await?;

    let client = super::cooking_builds::rpc_gateway_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.gateway.v1.GatewayService/ListProjectGateways",
    )?;
    let listed = client
        .list_project_gateways(ListProjectGatewaysRequest {
            project_id: super::cooking_builds::opaque(context.project_id.as_uuid()).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let gateway = listed
        .gateways
        .into_iter()
        .find(|gateway| {
            gateway
                .repository_id
                .as_option()
                .is_some_and(|id| id.value == published.repository_id.to_string())
        })
        .ok_or_else(|| std::io::Error::other("installed cooking service gateway was not listed"))?;
    let gateway_id = super::cooking_builds::response_id(
        gateway.id.into_option(),
        "ListProjectGateways cooking service gateway",
    )?;

    let client = super::cooking_builds::rpc_gateway_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.gateway.v1.GatewayService/GetGateway",
    )?;
    let current = client
        .get_gateway(GetGatewayRequest {
            gateway_id: super::cooking_builds::opaque(gateway_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let expected_revision_id = super::cooking_builds::response_id(
        current
            .gateway
            .into_option()
            .and_then(|summary| summary.desired_service_revision_id.into_option()),
        "GetGateway desired service revision",
    )?;

    let client = super::cooking_builds::rpc_gateway_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.gateway.v1.GatewayService/ConfigureGateway",
    )?;
    let configured = client
        .configure_gateway(ConfigureGatewayRequest {
            context: super::cooking_builds::mutation_context("configure-cooking-service").into(),
            gateway_id: super::cooking_builds::opaque(gateway_id).into(),
            expected_revision_id: super::cooking_builds::opaque(expected_revision_id).into(),
            parameters: Vec::new(),
            secret_selections: Vec::new(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let revision_id = super::cooking_builds::response_id(
        configured.revision_id.into_option(),
        "ConfigureGateway cooking service",
    )?;
    if configured.receipt.as_option().is_none() {
        return Err(std::io::Error::other("ConfigureGateway returned no receipt").into());
    }
    Ok(ConfiguredCookingService {
        gateway_id,
        revision_id,
    })
}
