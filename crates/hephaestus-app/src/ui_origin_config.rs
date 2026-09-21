//! Shared configuration and canonicalization for the private UI origin.

use release_service::{UiNamespace, UiPublicPort};
use std::net::SocketAddr;
use thiserror::Error;

/// Validated UI listener origin policy shared by bootstrap and content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiOriginConfig {
    namespace: UiNamespace,
    public_port: UiPublicPort,
    platform_origin: String,
    listener: Option<SocketAddr>,
}

impl UiOriginConfig {
    /// Validates the strict UI-subdomain and exact HTTPS platform-origin rule.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform origin is not canonical HTTPS or the
    /// UI namespace is not a strict subdomain of its host.
    pub fn new(
        namespace: UiNamespace,
        public_port: UiPublicPort,
        platform_origin: impl Into<String>,
    ) -> Result<Self, UiOriginConfigError> {
        let platform_origin = canonical_https_origin(&platform_origin.into())
            .ok_or(UiOriginConfigError::InvalidPlatformOrigin)?;
        let platform_host = platform_origin
            .strip_prefix("https://")
            .and_then(|value| {
                value
                    .split_once(':')
                    .map_or(Some(value), |(host, _)| Some(host))
            })
            .ok_or(UiOriginConfigError::InvalidPlatformOrigin)?;
        if namespace.as_str() == platform_host
            || !namespace.as_str().ends_with(&format!(".{platform_host}"))
        {
            return Err(UiOriginConfigError::NamespaceNotPlatformSubdomain);
        }
        Ok(Self {
            namespace,
            public_port,
            platform_origin,
            listener: None,
        })
    }

    /// Attaches the dedicated loopback listener address used by app wiring.
    #[must_use]
    pub const fn with_listener(mut self, listener: SocketAddr) -> Self {
        self.listener = Some(listener);
        self
    }

    /// Returns the configured listener when this value came from app config.
    #[must_use]
    pub const fn listener(&self) -> Option<SocketAddr> {
        self.listener
    }

    /// Returns the validated UI DNS namespace.
    #[must_use]
    pub const fn namespace(&self) -> &UiNamespace {
        &self.namespace
    }

    /// Returns the externally visible UI HTTPS port.
    #[must_use]
    pub const fn public_port(&self) -> UiPublicPort {
        self.public_port
    }

    /// Returns the canonical platform HTTPS origin.
    #[must_use]
    pub fn platform_origin(&self) -> &str {
        &self.platform_origin
    }
}

/// Configuration errors that must fail startup before Caddy is changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UiOriginConfigError {
    /// Platform origin is not a canonical HTTPS origin.
    #[error("platform origin must be an exact HTTPS origin")]
    InvalidPlatformOrigin,
    /// UI namespace is not a strict platform-host subdomain.
    #[error("UI namespace must be a strict subdomain of the platform host")]
    NamespaceNotPlatformSubdomain,
}

fn canonical_https_origin(value: &str) -> Option<String> {
    let authority = value.strip_prefix("https://")?;
    if authority.is_empty()
        || authority.bytes().any(|byte| {
            byte.is_ascii_control() || matches!(byte, b'@' | b'/' | b'?' | b'#' | b'\\')
        })
    {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            (host, Some(UiPublicPort::parse_text(port).ok()?))
        }
        Some(_) => return None,
        None => (authority, None),
    };
    let host = UiNamespace::parse(host.to_ascii_lowercase()).ok()?;
    if host.as_str().ends_with('.') || value != value.trim() {
        return None;
    }
    let port = port.unwrap_or_else(UiPublicPort::https_default);
    Some(if port.get() == 443 {
        format!("https://{}", host.as_str())
    } else {
        format!("https://{}:{}", host.as_str(), port.get())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_default_port_and_dns_case() {
        let config = UiOriginConfig::new(
            UiNamespace::parse("UI.Platform.Example.Test").expect("namespace"),
            UiPublicPort::parse(9443).expect("port"),
            "https://Platform.Example.Test:443",
        )
        .expect("origin");
        assert_eq!(config.platform_origin(), "https://platform.example.test");
    }

    #[test]
    fn keeps_ui_and_platform_ports_independent() {
        assert!(
            UiOriginConfig::new(
                UiNamespace::parse("ui.example.test").expect("namespace"),
                UiPublicPort::parse(9443).expect("port"),
                "https://example.test:8443",
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_aliases_and_non_subdomains() {
        let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
        let port = UiPublicPort::https_default();
        assert!(UiOriginConfig::new(namespace.clone(), port, "https://example.test/").is_err());
        assert!(UiOriginConfig::new(namespace, port, "https://other.test").is_err());
    }
}
