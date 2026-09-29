use release_service::UiBrowserSessionStore;
use release_service::ui_browser_host::{UiNamespace, UiPublicPort};
use release_service::ui_browser_serving::UiGenerationHostResolver;
use std::{sync::Arc, time::Duration};
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::{
    ui_audit::UiAuditRecorder,
    ui_origin_config::{UiOriginConfig, UiOriginConfigError},
};

/// The raw handoff is exactly 43 ASCII bytes. The extra allowance prevents a
/// transport implementation from buffering an unbounded malformed body.
pub const MAX_HANDOFF_BODY_BYTES: usize = 128;
/// Bootstrap must not leave a request waiting behind a stuck app-pool call.
pub const DEFAULT_BOOTSTRAP_DEADLINE: Duration = Duration::from_secs(5);
/// The listener has a separate bounded permit pool from the gateway worker.
pub const DEFAULT_BOOTSTRAP_CONCURRENCY: usize = 64;

/// Configuration that is safe to use in response headers and bootstrap HTML.
#[derive(Debug, Clone)]
pub struct UiBootstrapConfig {
    pub(super) origin: UiOriginConfig,
}

impl UiBootstrapConfig {
    /// Validates exact HTTPS origins and requires the UI namespace to be a
    /// strict subdomain of the configured platform host. This keeps the
    /// host-only cookie same-site without a public-suffix implementation.
    pub fn new(
        namespace: UiNamespace,
        public_port: UiPublicPort,
        platform_origin: impl Into<String>,
    ) -> Result<Self, UiBootstrapConfigError> {
        Ok(Self {
            origin: UiOriginConfig::new(namespace, public_port, platform_origin)
                .map_err(UiBootstrapConfigError::from)?,
        })
    }

    /// Returns the configured namespace.
    #[must_use]
    pub const fn namespace(&self) -> &UiNamespace {
        self.origin.namespace()
    }

    /// Returns the configured HTTPS public port.
    #[must_use]
    pub const fn public_port(&self) -> UiPublicPort {
        self.origin.public_port()
    }

    /// Returns the canonical platform HTTPS origin.
    #[must_use]
    pub fn platform_origin(&self) -> &str {
        self.origin.platform_origin()
    }
}

/// Configuration errors are kept separate from HTTP errors so startup can
/// fail closed instead of constructing a handler with unsafe origin policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UiBootstrapConfigError {
    /// Platform frame ancestors must be one exact HTTPS origin.
    #[error("platform origin must be an exact HTTPS origin")]
    InvalidPlatformOrigin,
    /// Cookie same-site policy requires a strict platform-host subdomain.
    #[error("UI namespace must be a strict subdomain of the platform host")]
    NamespaceNotPlatformSubdomain,
}

impl From<UiOriginConfigError> for UiBootstrapConfigError {
    fn from(error: UiOriginConfigError) -> Self {
        match error {
            UiOriginConfigError::InvalidPlatformOrigin => Self::InvalidPlatformOrigin,
            UiOriginConfigError::NamespaceNotPlatformSubdomain => {
                Self::NamespaceNotPlatformSubdomain
            }
        }
    }
}

/// Dependencies for the bootstrap route. The HTTP layer never receives a
/// database pool and never accepts actor, parent, organization, or generation
/// identity from the browser body.
pub struct UiBootstrapState {
    pub(super) host_resolver: Arc<dyn UiGenerationHostResolver>,
    pub(super) sessions: Arc<dyn UiBrowserSessionStore>,
    pub(super) config: UiBootstrapConfig,
    pub(super) permits: Arc<Semaphore>,
    pub(super) deadline: Duration,
    pub(super) audit: UiAuditRecorder,
}

impl UiBootstrapState {
    /// Constructs a bounded handler state for the private loopback listener.
    #[must_use]
    pub fn new(
        host_resolver: Arc<dyn UiGenerationHostResolver>,
        sessions: Arc<dyn UiBrowserSessionStore>,
        config: UiBootstrapConfig,
        audit_sink: Arc<dyn release_service::UiRequestAuditSink>,
    ) -> Self {
        Self {
            host_resolver,
            sessions,
            config,
            permits: Arc::new(Semaphore::new(DEFAULT_BOOTSTRAP_CONCURRENCY)),
            deadline: DEFAULT_BOOTSTRAP_DEADLINE,
            audit: UiAuditRecorder::new(audit_sink),
        }
    }

    /// Replaces the defaults for a focused listener test or an app-specific
    /// operational limit. A zero limit or zero deadline is rejected.
    #[allow(dead_code)] // Reserved for the daemon's listener composition layer.
    #[must_use]
    pub fn with_limits(mut self, concurrency: usize, deadline: Duration) -> Option<Self> {
        if concurrency == 0 || deadline.is_zero() {
            return None;
        }
        self.permits = Arc::new(Semaphore::new(concurrency));
        self.deadline = deadline;
        Some(self)
    }
}
