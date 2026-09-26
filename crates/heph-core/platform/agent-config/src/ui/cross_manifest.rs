use super::{RepositoryUisConfig, UiContent};
use crate::{Diagnostic, RepositoryGatewayConfig, RepositoryGatewaysConfig};
use gateway_domain::{Exposure, GatewayName, HttpMethod, RoutePath};

/// Validates UI references against an optional repository gateway manifest.
///
/// This is a source-only boundary check. It does not resolve gateway rows,
/// release artifacts, capability bindings, or runtime identities.
#[must_use]
pub fn validate_repository_uis_against_gateways(
    uis: &RepositoryUisConfig,
    gateways: Option<&RepositoryGatewaysConfig>,
) -> Vec<Diagnostic> {
    if let Some(invalid) = super::validation::validate(uis).first() {
        return vec![diagnostic(
            "invalid_repository_ui",
            ui_index_path(invalid.path.as_deref()),
            "repository UI declaration is invalid",
        )];
    }

    let first_required = uis
        .uis
        .iter()
        .enumerate()
        .find(|(_, ui)| requires_gateway(ui));
    let Some((first_index, _)) = first_required else {
        return Vec::new();
    };
    let Some(gateways) = gateways else {
        return vec![diagnostic(
            "missing_repository_ui_gateways",
            ui_path(first_index),
            "referenced gateway manifest is required",
        )];
    };
    if !crate::validate_repository_gateways(gateways).is_empty() {
        return vec![diagnostic(
            "invalid_repository_ui_gateways",
            ui_path(first_index),
            "referenced gateway manifest is invalid",
        )];
    }

    let mut diagnostics = Vec::new();
    for (ui_index, ui) in uis.uis.iter().enumerate() {
        for (api_index, api) in ui.apis.iter().enumerate() {
            let path = format!("uis[{ui_index}].apis[{api_index}]");
            let Some(gateway) = find_gateway(gateways, &api.gateway_name) else {
                diagnostics.push(diagnostic(
                    "missing_repository_ui_gateway",
                    path,
                    "UI API gateway reference is unavailable",
                ));
                continue;
            };
            validate_gateway_api(
                &mut diagnostics,
                ui_index,
                api_index,
                gateway,
                api.method,
                &api.route,
            );
        }
        if let UiContent::ManagedService {
            gateway_name,
            route,
            ..
        } = &ui.content
        {
            let path = format!("uis[{ui_index}].content");
            let Some(gateway) = find_gateway(gateways, gateway_name) else {
                diagnostics.push(diagnostic(
                    "missing_repository_ui_gateway",
                    path,
                    "managed UI gateway reference is unavailable",
                ));
                continue;
            };
            validate_managed_service(&mut diagnostics, ui_index, gateway, route);
        }
    }
    diagnostics
}

const fn requires_gateway(ui: &super::RepositoryUiConfig) -> bool {
    !ui.apis.is_empty() || matches!(&ui.content, UiContent::ManagedService { .. })
}

fn find_gateway<'a>(
    gateways: &'a RepositoryGatewaysConfig,
    requested: &GatewayName,
) -> Option<&'a RepositoryGatewayConfig> {
    gateways.gateways.iter().find(|gateway| {
        GatewayName::parse(gateway.name.clone()).is_ok_and(|name| name == *requested)
    })
}

fn validate_gateway_api(
    diagnostics: &mut Vec<Diagnostic>,
    ui_index: usize,
    api_index: usize,
    gateway: &RepositoryGatewayConfig,
    method: HttpMethod,
    route: &RoutePath,
) {
    let path = format!("uis[{ui_index}].apis[{api_index}]");
    if gateway.exposure != Exposure::HephAuthenticated {
        diagnostics.push(diagnostic(
            "repository_ui_gateway_not_authenticated",
            path.clone(),
            "UI gateway APIs require authenticated exposure",
        ));
    }
    if !gateway_covers(gateway, method, route) {
        diagnostics.push(diagnostic(
            "repository_ui_gateway_route_not_declared",
            path,
            "UI API route or method is not covered by the gateway declaration",
        ));
    }
}

fn validate_managed_service(
    diagnostics: &mut Vec<Diagnostic>,
    ui_index: usize,
    gateway: &RepositoryGatewayConfig,
    route: &RoutePath,
) {
    let path = format!("uis[{ui_index}].content");
    if gateway.exposure != Exposure::HephAuthenticated {
        diagnostics.push(diagnostic(
            "repository_ui_gateway_not_authenticated",
            path.clone(),
            "managed UI services require authenticated exposure",
        ));
    }
    if gateway.handler_contract != "http.service.v1" || gateway.service.is_none() {
        diagnostics.push(diagnostic(
            "repository_ui_managed_service_contract_required",
            path.clone(),
            "managed UI services require the versioned HTTP service contract",
        ));
    }
    if !gateway_covers(gateway, HttpMethod::Get, route) {
        diagnostics.push(diagnostic(
            "repository_ui_gateway_route_not_declared",
            path,
            "managed UI service route is not covered by an authenticated GET gateway route",
        ));
    }
}

fn gateway_covers(
    gateway: &RepositoryGatewayConfig,
    method: HttpMethod,
    selected: &RoutePath,
) -> bool {
    gateway.routes.iter().any(|route| {
        route.methods.contains(&method)
            && RoutePath::parse(route.path.clone())
                .is_ok_and(|declared| route_covers(declared.as_str(), selected.as_str()))
    })
}

fn route_covers(declared: &str, selected: &str) -> bool {
    selected == declared
        || selected
            .strip_prefix(declared)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn ui_path(index: usize) -> String {
    format!("uis[{index}]")
}

fn ui_index_path(path: Option<&str>) -> String {
    let Some(path) = path else {
        return String::from("uis");
    };
    let Some(rest) = path.strip_prefix("uis[") else {
        return String::from("uis");
    };
    let Some(end) = rest.find(']') else {
        return String::from("uis");
    };
    let index = &rest[..end];
    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        String::from("uis")
    } else {
        format!("uis[{index}]")
    }
}

fn diagnostic(code: &'static str, path: String, message: &'static str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        path: Some(path),
        message: message.into(),
    }
}
