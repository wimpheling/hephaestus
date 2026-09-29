//! Resolution of validated UI references to exact release-agent IDs.

use super::model::{
    GatewayReferenceKind, GatewayResolutionError, ReleaseAgentBinding, ResolvedApiBinding,
    ResolvedGatewayUi, ResolvedGatewayUis, ResolvedManagedService,
};
use crate::{
    RepositoryGatewaysConfig,
    ui::{RepositoryUisConfig, UiApiBinding, UiContent},
};
use gateway_domain::{GatewayName, RoutePath};
use release_domain::ui::UiRoutePath;
use release_domain::{AgentKey, ReleaseAgentId};
use std::collections::BTreeMap;

/// Resolves UI gateway references to caller-supplied exact release-agent IDs.
///
/// The optional canonical gateway configuration is required whenever a UI has
/// an API or managed-service reference. Static UIs without gateway references
/// resolve successfully with `None`. The source validator remains authoritative
/// for authenticated exposure, service contract, method, and route coverage.
///
/// # Errors
///
/// Returns a redacted, index-only error when source validation fails, supplied
/// agent bindings are ambiguous, or a referenced gateway/agent is unavailable.
pub fn resolve_gateway_uis(
    uis: &RepositoryUisConfig,
    gateways: Option<&RepositoryGatewaysConfig>,
    agents: &[ReleaseAgentBinding],
) -> Result<ResolvedGatewayUis, GatewayResolutionError> {
    let diagnostics = crate::ui::validate_repository_uis_against_gateways(uis, gateways);
    if !diagnostics.is_empty() {
        return Err(GatewayResolutionError::InvalidManifest {
            diagnostic_count: diagnostics.len(),
        });
    }

    let mut by_key: BTreeMap<AgentKey, (ReleaseAgentId, usize)> = BTreeMap::new();
    let mut by_id = BTreeMap::new();
    for (candidate_index, agent) in agents.iter().enumerate() {
        if let Some((_, first_index)) = by_key.insert(
            agent.agent_key.clone(),
            (agent.release_agent_id, candidate_index),
        ) {
            return Err(GatewayResolutionError::DuplicateAgentKey {
                candidate_index,
                first_index,
            });
        }
        if let Some(first_index) = by_id.insert(agent.release_agent_id, candidate_index) {
            return Err(GatewayResolutionError::DuplicateAgentId {
                candidate_index,
                first_index,
            });
        }
    }

    let mut resolved = Vec::with_capacity(uis.uis.len());
    for (ui_index, ui) in uis.uis.iter().enumerate() {
        let managed_service = match &ui.content {
            UiContent::ManagedService {
                gateway_name,
                route,
                entrypoint,
            } => Some(resolve_managed_service(
                gateways,
                &by_key,
                ui_index,
                gateway_name,
                route,
                entrypoint,
            )?),
            UiContent::Static { .. } => None,
        };
        let api_bindings = ui
            .apis
            .iter()
            .enumerate()
            .map(|(api_index, api)| resolve_api(gateways, &by_key, ui_index, api_index, api))
            .collect::<Result<Vec<_>, _>>()?;
        resolved.push(ResolvedGatewayUi {
            key: ui.key.clone(),
            managed_service,
            apis: api_bindings,
        });
    }
    Ok(ResolvedGatewayUis { uis: resolved })
}

fn resolve_managed_service(
    gateways: Option<&RepositoryGatewaysConfig>,
    agents: &BTreeMap<AgentKey, (ReleaseAgentId, usize)>,
    ui_index: usize,
    gateway_name: &GatewayName,
    route: &RoutePath,
    entrypoint: &UiRoutePath,
) -> Result<ResolvedManagedService, GatewayResolutionError> {
    let gateway =
        find_gateway(gateways, gateway_name).ok_or(GatewayResolutionError::MissingGateway {
            ui_index,
            reference_kind: GatewayReferenceKind::ManagedService,
            reference_index: 0,
        })?;
    let agent_id = resolve_agent_id(
        agents,
        &gateway.agent_name,
        ui_index,
        GatewayReferenceKind::ManagedService,
        0,
    )?;
    Ok(ResolvedManagedService {
        gateway_name: gateway_name.clone(),
        release_agent_id: agent_id,
        route: route.clone(),
        entrypoint: entrypoint.clone(),
    })
}

fn resolve_api(
    gateways: Option<&RepositoryGatewaysConfig>,
    agents: &BTreeMap<AgentKey, (ReleaseAgentId, usize)>,
    ui_index: usize,
    api_index: usize,
    api: &UiApiBinding,
) -> Result<ResolvedApiBinding, GatewayResolutionError> {
    let gateway = find_gateway(gateways, &api.gateway_name).ok_or(
        GatewayResolutionError::MissingGateway {
            ui_index,
            reference_kind: GatewayReferenceKind::Api,
            reference_index: api_index,
        },
    )?;
    let agent_id = resolve_agent_id(
        agents,
        &gateway.agent_name,
        ui_index,
        GatewayReferenceKind::Api,
        api_index,
    )?;
    Ok(ResolvedApiBinding {
        key: api.key.clone(),
        gateway_name: api.gateway_name.clone(),
        release_agent_id: agent_id,
        method: api.method,
        route: api.route.clone(),
    })
}

fn find_gateway<'a>(
    gateways: Option<&'a RepositoryGatewaysConfig>,
    requested: &GatewayName,
) -> Option<&'a crate::RepositoryGatewayConfig> {
    gateways?.gateways.iter().find(|gateway| {
        GatewayName::parse(gateway.name.clone()).is_ok_and(|name| name == *requested)
    })
}

fn resolve_agent_id(
    agents: &BTreeMap<AgentKey, (ReleaseAgentId, usize)>,
    agent_name: &str,
    ui_index: usize,
    reference_kind: GatewayReferenceKind,
    reference_index: usize,
) -> Result<ReleaseAgentId, GatewayResolutionError> {
    let Ok(agent_key) = AgentKey::parse(agent_name.to_owned()) else {
        return Err(GatewayResolutionError::MissingAgent {
            ui_index,
            reference_kind,
            reference_index,
        });
    };
    let Some(&(release_agent_id, _candidate_index)) = agents.get(&agent_key) else {
        return Err(GatewayResolutionError::MissingAgent {
            ui_index,
            reference_kind,
            reference_index,
        });
    };
    Ok(release_agent_id)
}
