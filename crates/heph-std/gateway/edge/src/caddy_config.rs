use serde_json::Value;
use std::net::SocketAddr;

use crate::{Exposure, GatewayDesiredConfiguration, GatewayEdgeError};

/// Complete shared-Caddy configuration with one dedicated gateway subroute.
///
/// Caddy's `/load` endpoint atomically replaces its whole configuration.  To
/// avoid clobbering platform-owned routes, an operator supplies that complete
/// baseline and explicitly reserves one `group = "hephaestus.gateway"`
/// subroute. Reconciliation replaces only that subroute's nested `routes` in
/// an in-memory copy before atomically loading it.
#[derive(Clone)]
pub struct LocalCaddyConfigurationTemplate {
    base: Value,
    server: String,
    ui_namespace: Option<UiNamespaceConfiguration>,
}

#[derive(Clone)]
struct UiNamespaceConfiguration {
    namespace: String,
    upstream: SocketAddr,
}

impl LocalCaddyConfigurationTemplate {
    /// Parses and validates a complete Caddy JSON configuration.
    ///
    /// The selected server must contain exactly one route with
    /// `group: "hephaestus.gateway"` and one `subroute` handler. Platform
    /// routes remain elsewhere in the complete supplied configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the bytes are not a complete Caddy JSON
    /// configuration or do not reserve exactly one valid gateway subroute.
    pub fn new(configuration: &[u8], server: String) -> Result<Self, GatewayEdgeError> {
        if server.is_empty() {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        }
        let base: Value = serde_json::from_slice(configuration)
            .map_err(|_| GatewayEdgeError::InvalidCaddyConfiguration)?;
        let template = Self {
            base,
            server,
            ui_namespace: None,
        };
        template.gateway_subroute_index()?;
        Ok(template)
    }

    /// Enables the optional terminal UI namespace route.
    ///
    /// The operator baseline must reserve exactly one top-level
    /// `group = "hephaestus.ui"` route at index zero. The private upstream is
    /// restricted to a loopback socket; the UI service remains responsible for
    /// canonical host-to-generation resolution and unknown-host denial.
    ///
    /// # Errors
    ///
    /// Returns an error when the namespace, reserved Caddy slot, or upstream
    /// socket is invalid.
    pub fn with_ui_namespace(
        mut self,
        namespace: &str,
        upstream: SocketAddr,
    ) -> Result<Self, GatewayEdgeError> {
        let namespace = namespace.to_ascii_lowercase();
        validate_ui_namespace(&namespace)?;
        if !upstream.ip().is_loopback() || upstream.port() == 0 {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        }
        self.ui_namespace_slot_index()?;
        self.ui_namespace = Some(UiNamespaceConfiguration {
            namespace,
            upstream,
        });
        Ok(self)
    }

    fn ui_namespace_slot_index(&self) -> Result<usize, GatewayEdgeError> {
        let routes = self.server_routes()?;
        let indices: Vec<_> = routes
            .iter()
            .enumerate()
            .filter_map(|(index, route)| {
                (route.get("group").and_then(Value::as_str) == Some("hephaestus.ui"))
                    .then_some(index)
            })
            .collect();
        let [index] = indices.as_slice() else {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        };
        if *index != 0 {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        }
        let handler = routes[*index]
            .get("handle")
            .and_then(Value::as_array)
            .and_then(|handlers| handlers.first())
            .filter(|handler| handler.get("handler").and_then(Value::as_str) == Some("subroute"));
        if handler.is_none() {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        }
        Ok(*index)
    }

    fn gateway_subroute_index(&self) -> Result<usize, GatewayEdgeError> {
        let routes = self.server_routes()?;
        let indices: Vec<_> = routes
            .iter()
            .enumerate()
            .filter_map(|(index, route)| {
                (route.get("group").and_then(Value::as_str) == Some("hephaestus.gateway"))
                    .then_some(index)
            })
            .collect();
        let [index] = indices.as_slice() else {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        };
        let handler = routes[*index]
            .get("handle")
            .and_then(Value::as_array)
            .and_then(|handlers| handlers.first())
            .filter(|handler| handler.get("handler").and_then(Value::as_str) == Some("subroute"));
        if handler.is_none() {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        }
        Ok(*index)
    }

    fn server_routes(&self) -> Result<&Vec<Value>, GatewayEdgeError> {
        self.base
            .pointer(&format!("/apps/http/servers/{}/routes", self.server))
            .and_then(Value::as_array)
            .ok_or(GatewayEdgeError::InvalidCaddyConfiguration)
    }

    pub(crate) fn render(
        &self,
        desired: &GatewayDesiredConfiguration,
        dispatcher_upstream: &str,
    ) -> Result<Vec<u8>, GatewayEdgeError> {
        if dispatcher_upstream.is_empty() {
            return Err(GatewayEdgeError::Unavailable);
        }
        let mut configuration = self.base.clone();
        if let Some(ui_namespace) = &self.ui_namespace {
            let slot = self.ui_namespace_slot_index()?;
            let pointer = format!("/apps/http/servers/{}/routes", self.server);
            let target = configuration
                .pointer_mut(&pointer)
                .and_then(Value::as_array_mut)
                .ok_or(GatewayEdgeError::InvalidCaddyConfiguration)?;
            target[slot] = caddy_ui_namespace_route(ui_namespace);
        }
        let slot = self.gateway_subroute_index()?;
        let routes = caddy_gateway_routes(desired, dispatcher_upstream);
        let pointer = format!(
            "/apps/http/servers/{}/routes/{slot}/handle/0/routes",
            self.server
        );
        let target = configuration
            .pointer_mut(&pointer)
            .ok_or(GatewayEdgeError::InvalidCaddyConfiguration)?;
        *target = Value::Array(routes);
        serde_json::to_vec(&configuration).map_err(|_| GatewayEdgeError::Unavailable)
    }
}

fn validate_ui_namespace(namespace: &str) -> Result<(), GatewayEdgeError> {
    if namespace.is_empty() || namespace.len() > 253 || namespace.ends_with('.') {
        return Err(GatewayEdgeError::InvalidCaddyConfiguration);
    }
    for label in namespace.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        }
    }
    Ok(())
}

fn caddy_ui_namespace_route(configuration: &UiNamespaceConfiguration) -> Value {
    let suffix = configuration.namespace.replace('.', "[.]");
    let pattern = format!("(?i)^(?:.*[.])?{suffix}[.]?(?::[0-9]{{1,5}})?$");
    serde_json::json!({
        "group": "hephaestus.ui",
        "match": [{
            "expression": {
                "name": "ui_namespace",
                "expr": format!("header_regexp('Host', '{pattern}')")
            }
        }],
        "handle": [{
            "handler": "reverse_proxy",
            "upstreams": [{ "dial": configuration.upstream.to_string() }]
        }],
        "terminal": true
    })
}

pub fn caddy_gateway_routes(
    desired: &GatewayDesiredConfiguration,
    dispatcher_upstream: &str,
) -> Vec<Value> {
    let mut routes = desired.routes.clone();
    routes.sort_by(|left, right| left.path_prefix.cmp(&right.path_prefix));
    routes
        .into_iter()
        .filter(|route| route.exposure == Exposure::Public)
        .map(|route| {
            let path = route.public_path();
            serde_json::json!({
                "match": [{ "path": [path.clone(), format!("{path}/*")] }],
                "handle": [{
                    "handler": "reverse_proxy",
                    "upstreams": [{ "dial": dispatcher_upstream }]
                }],
                "terminal": true
            })
        })
        .collect()
}
