use super::{
    common::{
        BrokerAdapterError, BrokerResponse, BrokeredSecretRule, HashMap, IpAddr, async_trait,
    },
    transport::ReqwestPinnedHttpsTransport,
};
use serde::{Deserialize, Serialize};

/// Guest-provided HTTP request transported over the broker vsock channel.
///
/// This is the locked-plan cooperating transport: the guest has no IP
/// interface in `BrokerOnly` mode, so it cannot bypass this host-side TLS
/// client or observe the substituted credential.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokeredHttpsRequest {
    /// Immutable rule claimed by this request. The runtime resolver verifies
    /// it against the exact lease snapshot before it decrypts a secret.
    pub rule_id: uuid::Uuid,
    /// Bounded HTTP method. The MVP keeps this intentionally small.
    pub method: BrokeredHttpsMethod,
    /// Origin-relative path and optional query, never an absolute URL.
    pub path_and_query: String,
    /// Request headers. Header names are lower-case ASCII.
    pub headers: Vec<BrokeredHttpsHeader>,
    /// Bounded request body.
    pub body: Vec<u8>,
}

/// Supported ordinary HTTPS methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokeredHttpsMethod {
    /// Read an endpoint.
    Get,
    /// Submit a bounded request.
    Post,
    /// Replace a bounded endpoint value.
    Put,
    /// Remove an endpoint value.
    Delete,
}

/// One request header, free of credentials until the host applies its rule.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokeredHttpsHeader {
    /// Lower-case ASCII header name.
    pub name: String,
    /// Header value with no line breaks or NUL bytes.
    pub value: String,
}

/// Host-side request after exact placeholder substitution.
///
/// This type intentionally has no `Debug` implementation because one header
/// may contain plaintext resolved only inside the secret authority boundary.
pub struct UpstreamHttpsRequest {
    pub(super) method: BrokeredHttpsMethod,
    pub(super) destination: String,
    pub(super) path_and_query: String,
    pub(super) headers: Vec<BrokeredHttpsHeader>,
    pub(super) body: Vec<u8>,
}

impl UpstreamHttpsRequest {
    /// Returns the fixed destination for a trusted host transport.
    #[must_use]
    pub fn destination(&self) -> &str {
        &self.destination
    }

    /// Returns the validated relative request path.
    #[must_use]
    pub fn path_and_query(&self) -> &str {
        &self.path_and_query
    }
}

/// Host-only HTTPS transport. Implementations must make a verified TLS
/// connection to the exact `destination` and must not follow redirects.
#[async_trait]
pub trait PinnedHttpsTransport: Send + Sync + 'static {
    /// Sends one already-authorized and substituted request.
    async fn send(
        &self,
        request: UpstreamHttpsRequest,
    ) -> Result<BrokerResponse, BrokerAdapterError>;
}

/// Generic adapter that turns an ordinary bounded HTTPS request into one
/// brokered secret use without provider-specific semantics.
pub struct BrokeredHttpsAdapter<T> {
    pub(super) destination: String,
    pub(super) rule: BrokeredSecretRule,
    pub(super) transport: T,
}

/// One operator-pinned HTTPS upstream made available to the daemon.
///
/// The rule is immutable, non-secret release authority. The addresses are
/// resolved and approved by the control plane, never supplied by a guest.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokeredHttpsUpstream {
    /// Exact immutable rule that may be used through this upstream.
    pub rule: BrokeredSecretRule,
    /// Non-private DNS addresses pinned for this exact rule destination.
    pub addresses: Vec<IpAddr>,
}

/// One operator-pinned HTTPS origin transport usable by dynamically declared
/// immutable rules.
///
/// Rule identity and header authority still come from the verified runtime
/// lease projection; this entry only permits the origin and supplies its
/// control-plane-pinned addresses.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokeredHttpsOrigin {
    /// Exact HTTPS origin permitted for dynamic rules.
    pub origin: String,
    /// Non-private DNS addresses pinned for this origin.
    pub addresses: Vec<IpAddr>,
}

pub(super) struct OriginPinnedHttpsAdapter {
    pub(super) destination: String,
    pub(super) transport: ReqwestPinnedHttpsTransport,
}

/// Daemon adapter registry for all explicitly configured brokered HTTPS rules.
///
/// A request is selected solely by its claimed immutable rule ID. The runtime
/// resolver independently verifies that ID against the exact issued lease
/// before this registry receives the resolved credential.
pub struct BrokeredHttpsAdapterRegistry {
    pub(super) adapters: HashMap<uuid::Uuid, BrokeredHttpsAdapter<ReqwestPinnedHttpsTransport>>,
    pub(super) origin_adapters: HashMap<String, OriginPinnedHttpsAdapter>,
}
