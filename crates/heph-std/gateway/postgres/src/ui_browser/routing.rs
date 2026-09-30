//! Conversion from an authorized database binding to a gateway admission.

use super::authorization::UiAuthorizedRequest;
use crate::PostgresGatewayEdgeAuthority;
use gateway_domain::Exposure;
use gateway_domain::{
    GATEWAY_NAMESPACE, GatewayEdgeError, GatewayRouteBinding, UiGatewayAdmission,
    UiGatewayAuthority, UiGatewayRequestKind,
};
use http::Method;
use std::collections::BTreeSet;

pub(super) fn build_admission(
    authority: &PostgresGatewayEdgeAuthority,
    safe_authority: &UiGatewayAuthority,
    request_path_and_query: &str,
    authorized: &UiAuthorizedRequest,
) -> Result<UiGatewayAdmission, GatewayEdgeError> {
    if authorized.binding.exposure != "heph_authenticated"
        || (authorized.binding.handler_contract != "http.v1"
            && authorized.binding.handler_contract != "http.service.v1")
    {
        return Err(GatewayEdgeError::Unavailable);
    }
    let path = path_component(request_path_and_query);
    let gateway_path = match safe_authority.request_kind {
        UiGatewayRequestKind::Managed => {
            let suffix = path
                .strip_prefix(&authorized.binding.route_base)
                .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'))
                .ok_or(GatewayEdgeError::Unavailable)?;
            format!("{}{}", authorized.binding.binding_route, suffix)
        }
        UiGatewayRequestKind::Api => {
            if path != authorized.binding.binding_route {
                return Err(GatewayEdgeError::Unavailable);
            }
            authorized.binding.binding_route.clone()
        }
    };
    let methods = authorized
        .binding
        .gateway_methods
        .iter()
        .map(|value| persisted_method(value))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let path_prefix = authorized
        .binding
        .gateway_path
        .strip_prefix('/')
        .ok_or(GatewayEdgeError::Unavailable)?
        .to_owned();
    let route = GatewayRouteBinding {
        route_id: authorized.binding.route_id,
        gateway_revision_id: authorized.binding.gateway_revision_id,
        exposure: Exposure::HephAuthenticated,
        path_prefix,
        methods,
        limits: authority.limits,
    };
    route.validate()?;
    let query = request_path_and_query
        .split_once('?')
        .map(|(_, query)| query);
    let gateway_path = format!(
        "{GATEWAY_NAMESPACE}{}",
        gateway_path.trim_start_matches('/')
    );
    let gateway_path_and_query = match query {
        Some(query) => format!("{gateway_path}?{query}"),
        None => gateway_path,
    };
    Ok(UiGatewayAdmission {
        route,
        gateway_path_and_query,
    })
}

pub(super) fn path_component(path_and_query: &str) -> &str {
    path_and_query
        .split_once('?')
        .map_or(path_and_query, |(path, _)| path)
}

pub(super) const fn sql_request_kind(kind: UiGatewayRequestKind) -> &'static str {
    match kind {
        UiGatewayRequestKind::Managed => "managed_service",
        UiGatewayRequestKind::Api => "api",
    }
}

pub(super) fn persisted_method(value: &str) -> Result<Method, GatewayEdgeError> {
    match value {
        "GET" => Ok(Method::GET),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        "HEAD" => Ok(Method::HEAD),
        "OPTIONS" => Ok(Method::OPTIONS),
        _ => Err(GatewayEdgeError::Unavailable),
    }
}
