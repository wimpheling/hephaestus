//! Loopback-only Caddy administration adapter.

use crate::{CaddyAdministration, GatewayEdgeError};
use async_trait::async_trait;
use std::net::IpAddr;

/// Loopback-only HTTP client for Caddy's private administration API.
///
/// This adapter deliberately exposes only whole-config `POST /load`. Gateway
/// VMs never receive its endpoint or client, and no caller can use it to issue
/// arbitrary Caddy administration requests.
#[derive(Clone)]
pub struct LocalCaddyAdministration {
    client: reqwest::Client,
    load_endpoint: reqwest::Url,
}

impl LocalCaddyAdministration {
    /// Creates an administration client for a loopback Caddy admin endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error unless `endpoint` is an HTTP URL addressed exactly to
    /// an IP loopback host. A public or DNS administration endpoint would make
    /// the shared edge control plane remotely mutable.
    pub fn new(endpoint: &str) -> Result<Self, GatewayEdgeError> {
        let mut endpoint = reqwest::Url::parse(endpoint)
            .map_err(|_| GatewayEdgeError::InvalidAdministrationEndpoint)?;
        let loopback = endpoint
            .host_str()
            .and_then(|host| host.parse::<IpAddr>().ok())
            .is_some_and(|address| address.is_loopback());
        if endpoint.scheme() != "http" || !loopback {
            return Err(GatewayEdgeError::InvalidAdministrationEndpoint);
        }
        endpoint.set_path("/load");
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        Ok(Self {
            client: reqwest::Client::new(),
            load_endpoint: endpoint,
        })
    }
}

#[async_trait]
impl CaddyAdministration for LocalCaddyAdministration {
    async fn load(&self, configuration: Vec<u8>) -> Result<(), GatewayEdgeError> {
        let response = self
            .client
            .post(self.load_endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(configuration)
            .send()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(GatewayEdgeError::Unavailable)
        }
    }
}
