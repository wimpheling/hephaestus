//! Provider-neutral browser-session application commands and persistence ports.

use async_trait::async_trait;
use identity_domain::{BrowserSessionMetadata, BrowserSessionSid, RequestId, UserId};

/// External identity fields bound to a verified bootstrap assertion.
///
/// This type intentionally carries only the issuer and subject needed for a
/// browser-session lookup. It does not carry actor selectors, provider claims,
/// or display data.
pub struct VerifiedBrowserIdentity {
    /// Verified identity-provider issuer.
    pub issuer: String,
    /// Verified issuer-local subject.
    pub subject: String,
}

/// Idempotent creation of one browser session from a verified OIDC identity.
pub struct CreateBrowserSession {
    /// Correlation identifier for this transport request.
    pub request_id: RequestId,
    /// Method-scoped digest of the caller's idempotency key.
    pub idempotency_seed: [u8; 32],
    /// Issuer and subject authenticated by the bootstrap assertion.
    pub verified: VerifiedBrowserIdentity,
    /// Request-only bearer value; implementations persist only its digest.
    pub sid: BrowserSessionSid,
}

/// Safe result of browser-session creation or an active exact replay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreatedBrowserSession {
    /// Durable session metadata, never the raw SID.
    pub metadata: BrowserSessionMetadata,
    /// Actor-bound operation ID used to load the mutation receipt.
    pub idempotency_id: RequestId,
}

/// Self-revocation authenticated by the signed mediator user and SID claims.
///
/// The store treats an absent, expired, or already-revoked session as an
/// idempotent no-op. This command is deliberately separate from active-session
/// authentication and carries no caller-selected revocation reason.
pub struct RevokeBrowserSession {
    /// Correlation identifier for this transport request.
    pub request_id: RequestId,
    /// Method-scoped digest of the caller's idempotency key.
    pub idempotency_seed: [u8; 32],
    /// User ID taken from the verified mediator assertion.
    pub user_id: UserId,
    /// SID taken from the verified mediator assertion.
    pub sid: BrowserSessionSid,
}

/// Safe result of a self-revocation attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RevokedBrowserSession {
    /// Actor-bound operation ID used for audit and receipt lookup.
    pub idempotency_id: RequestId,
    /// Internal persistence result; transport responses must not expose it.
    pub changed: bool,
}

/// Typed failures from browser-session creation.
#[derive(Debug, thiserror::Error)]
pub enum CreateBrowserSessionError {
    /// The verified external identity is not mapped to an active user.
    #[error("browser session identity is not permitted")]
    PermissionDenied,
    /// The idempotency key was reused with different identity or SID data.
    #[error("browser session creation conflicts with an earlier retry")]
    IdempotencyConflict,
    /// The exact creation already exists but is no longer active.
    #[error("browser session is no longer active")]
    InactiveReplay,
    /// The persistence provider failed without exposing request material.
    #[error("browser session persistence is unavailable")]
    Unavailable,
}

/// Typed failures from active browser-session verification.
#[derive(Debug, thiserror::Error)]
pub enum BrowserSessionAuthenticationError {
    /// The SID does not identify an active session for this user.
    #[error("browser session authentication failed")]
    Unauthenticated,
    /// The persistence provider failed without exposing session material.
    #[error("browser session persistence is unavailable")]
    Unavailable,
}

/// Typed failures from browser-session self-revocation.
#[derive(Debug, thiserror::Error)]
pub enum RevokeBrowserSessionError {
    /// The signed user does not exist in the current identity store.
    #[error("browser session user is not authenticated")]
    Unauthenticated,
    /// The idempotency key was reused with a different user or SID.
    #[error("browser session revocation conflicts with an earlier retry")]
    IdempotencyConflict,
    /// The persistence provider failed without exposing session material.
    #[error("browser session persistence is unavailable")]
    Unavailable,
}

/// SQL-free persistence port for browser-session lifecycle operations.
#[async_trait]
pub trait BrowserSessionStore: Send + Sync {
    /// Resolves the verified identity, creates or replays an active session,
    /// and returns only safe metadata plus the receipt lookup identifier.
    async fn create_browser_session(
        &self,
        command: CreateBrowserSession,
    ) -> Result<CreatedBrowserSession, CreateBrowserSessionError>;

    /// Authenticates one active session for the exact signed user and SID.
    async fn authenticate_browser_session(
        &self,
        user_id: UserId,
        sid: BrowserSessionSid,
    ) -> Result<BrowserSessionMetadata, BrowserSessionAuthenticationError>;

    /// Revokes only the exact signed user's SID. Missing, expired, and already
    /// revoked rows are successful no-ops.
    async fn revoke_browser_session(
        &self,
        command: RevokeBrowserSession,
    ) -> Result<RevokedBrowserSession, RevokeBrowserSessionError>;
}
