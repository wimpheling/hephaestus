//! Provider-neutral broker request and adapter contracts.

use async_trait::async_trait;
use runtime_types::RunId;
use secret_domain::{SecretSlotKey, SecretValue};
use uuid::Uuid;

/// Bounded semantic broker request from one exact runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerRequest {
    /// Claimed run identity, matched against the opaque credential.
    pub run_id: RunId,
    /// Symbolic released slot.
    pub slot: SecretSlotKey,
    /// Exact allowlisted destination name.
    pub destination: String,
    /// Adapter-defined semantic operation, never an arbitrary URL.
    pub operation: String,
    /// Bounded application body.
    pub body: Vec<u8>,
}

/// Immutable HTTPS rule metadata verified against one runtime lease snapshot.
///
/// The runtime service constructs this projection only after checking the
/// claimed rule, binding, version, session, run, and destination. Host
/// adapters may use it for header substitution and transport selection; they
/// must not derive authority from the guest request body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBrokeredHttpsRule {
    /// Immutable placeholder rule identity.
    pub rule_id: Uuid,
    /// Exact HTTPS origin, including its scheme.
    pub destination_origin: String,
    /// Exact outbound header name.
    pub header_name: String,
    /// Optional fixed value prefix before the runtime credential.
    pub header_prefix: Option<String>,
}

/// Sanitized broker response without upstream headers or credential material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerResponse {
    /// Provider-neutral result status.
    pub status: BrokerStatus,
    /// Bounded adapter-sanitized response body.
    pub body: Vec<u8>,
}

/// Provider-neutral broker outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerStatus {
    /// Semantic operation completed.
    Succeeded,
    /// Upstream rejected the semantic operation.
    Rejected,
    /// A retry may succeed without changing the request.
    Retryable,
}

/// Host-only application adapter. Implementations own DNS, transport,
/// redirect, metadata-endpoint, and response sanitization policy.
#[async_trait]
pub trait BrokerAdapter: Send + Sync {
    /// Applies the exact credential to one semantic allowlisted operation.
    ///
    /// Implementations must not return upstream authorization headers,
    /// credential-bearing redirects, or raw provider error bodies.
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError>;

    /// Applies a verified generic HTTPS rule to one bounded request.
    ///
    /// The default preserves compatibility with legacy adapters. Generic
    /// HTTPS adapters that support rules declared after startup override this
    /// seam and consume the verified projection explicitly.
    async fn invoke_verified_https(
        &self,
        credential: &SecretValue,
        request: &BrokerRequest,
        _rule: &VerifiedBrokeredHttpsRule,
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        self.invoke(
            credential,
            &request.destination,
            &request.operation,
            &request.body,
        )
        .await
    }
}

/// Sanitized adapter failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BrokerAdapterError {
    /// The semantic operation is not allowed.
    #[error("broker operation is rejected")]
    Rejected,
    /// The provider may be retried.
    #[error("broker provider is temporarily unavailable")]
    Retryable,
}
