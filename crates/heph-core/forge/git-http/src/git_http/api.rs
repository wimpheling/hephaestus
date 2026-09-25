use crate::{AuthenticationError, AuthorizationError, receive_policy};
use async_trait::async_trait;
use forge_domain::RepositoryId;
use identity_domain::{AuthenticatedIdentity, RequestId};
use runtime_git_authority::AuthenticatedRuntimeGitAuthority;

/// Permission checked for a Git transport operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitOperation {
    /// Repository discovery for clone.
    Clone,
    /// Object transfer after discovery.
    Fetch,
    /// Reference advertisement or object transfer for push.
    Push,
}

/// Authenticated principal returned by a Git credential authenticator.
///
/// Human and runtime identities remain distinct so a runtime credential can
/// never accidentally inherit authorization through a human identity path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Principal {
    /// A human authenticated through OIDC or a future user credential.
    Human(HumanPrincipal),
    /// One exact runtime session authenticated by a future runtime credential.
    Runtime(RuntimePrincipal),
}

impl Principal {
    /// Creates a human principal from a verified internal identity.
    #[must_use]
    pub fn human(identity: AuthenticatedIdentity) -> Self {
        Self::Human(HumanPrincipal {
            name: identity.subject.clone(),
            identity,
        })
    }

    /// Creates an exact-runtime principal from opaque control-plane IDs.
    #[must_use]
    pub fn runtime(
        name: impl Into<String>,
        runtime_session_id: impl Into<String>,
        authorization_snapshot_id: impl Into<String>,
    ) -> Self {
        Self::Runtime(RuntimePrincipal {
            name: name.into(),
            runtime_session_id: runtime_session_id.into(),
            authorization_snapshot_id: authorization_snapshot_id.into(),
            receive_context: None,
            git_authority: None,
        })
    }

    /// Creates an exact-runtime principal carrying host-resolved authority for
    /// Git's quarantined pre-receive boundary.
    #[must_use]
    pub fn runtime_with_receive_context(
        name: impl Into<String>,
        receive_context: receive_policy::ResolvedRuntimeReceiveContext,
    ) -> Self {
        Self::Runtime(RuntimePrincipal {
            name: name.into(),
            runtime_session_id: receive_context.runtime_session_id().to_owned(),
            authorization_snapshot_id: receive_context.authorization_snapshot_id().to_owned(),
            receive_context: Some(receive_context),
            git_authority: None,
        })
    }

    /// Creates an exact runtime principal from host-authenticated Git
    /// authority.
    #[must_use]
    pub fn runtime_with_git_authority(
        name: impl Into<String>,
        authority: AuthenticatedRuntimeGitAuthority,
    ) -> Self {
        Self::Runtime(RuntimePrincipal {
            name: name.into(),
            runtime_session_id: authority.runtime_session_id.to_string(),
            authorization_snapshot_id: authority.authorization_snapshot_id.to_string(),
            receive_context: None,
            git_authority: Some(authority),
        })
    }

    /// Returns the stable provider-neutral principal name exposed to Git.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Human(principal) => &principal.name,
            Self::Runtime(principal) => &principal.name,
        }
    }

    /// Returns a verified human identity, if this is a human principal.
    #[must_use]
    pub const fn human_identity(&self) -> Option<&AuthenticatedIdentity> {
        match self {
            Self::Human(principal) => Some(&principal.identity),
            Self::Runtime(_) => None,
        }
    }
}

/// A verified human Git principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanPrincipal {
    name: String,
    identity: AuthenticatedIdentity,
}

/// An exact runtime Git principal whose identifiers remain opaque to transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePrincipal {
    name: String,
    runtime_session_id: String,
    authorization_snapshot_id: String,
    receive_context: Option<receive_policy::ResolvedRuntimeReceiveContext>,
    git_authority: Option<AuthenticatedRuntimeGitAuthority>,
}

impl RuntimePrincipal {
    /// Returns the opaque runtime-session identifier.
    #[must_use]
    pub fn runtime_session_id(&self) -> &str {
        &self.runtime_session_id
    }

    /// Returns the opaque immutable authorization-snapshot identifier.
    #[must_use]
    pub fn authorization_snapshot_id(&self) -> &str {
        &self.authorization_snapshot_id
    }

    /// Returns host-resolved receive authority when this runtime credential
    /// permits a quarantined receive.
    #[must_use]
    pub const fn receive_context(&self) -> Option<&receive_policy::ResolvedRuntimeReceiveContext> {
        self.receive_context.as_ref()
    }

    /// Returns complete host-resolved Git authority for this request.
    #[must_use]
    pub const fn git_authority(&self) -> Option<&AuthenticatedRuntimeGitAuthority> {
        self.git_authority.as_ref()
    }
}

/// Owned authorization input.
#[derive(Debug, Clone)]
pub struct AuthorizationRequest {
    /// Repository being accessed.
    pub repository_id: RepositoryId,
    /// Requested operation.
    pub operation: GitOperation,
    /// Principal established by the HTTP authentication middleware.
    pub principal: Principal,
}

/// Authentication boundary used by Git HTTP middleware.
#[async_trait]
pub trait GitAuthenticator: Send + Sync + 'static {
    /// Consumes an HTTP credential and returns a verified request principal.
    ///
    /// # Errors
    ///
    /// Returns a non-sensitive error when the credential cannot be verified or
    /// mapped to an active internal user.
    async fn authenticate(
        &self,
        credential: Option<&str>,
        request_id: RequestId,
    ) -> Result<Principal, AuthenticationError>;

    /// Consumes a Git HTTP credential bound to its exact repository operation.
    ///
    /// Authenticators without token-local Git scope delegate to
    /// [`Self::authenticate`]. Scoped credentials override this method so the
    /// credential cannot be accepted before its repository and operation are
    /// known.
    ///
    /// # Errors
    ///
    /// Returns a non-sensitive error when the credential is invalid for the
    /// exact repository operation.
    async fn authenticate_git(
        &self,
        credential: Option<&str>,
        request_id: RequestId,
        repository_id: RepositoryId,
        operation: GitOperation,
    ) -> Result<Principal, AuthenticationError> {
        let _ = (repository_id, operation);
        self.authenticate(credential, request_id).await
    }
}

/// Authorization boundary for every Git operation.
#[async_trait]
pub trait GitAuthorizer: Send + Sync + 'static {
    /// Authorizes one operation and returns its authenticated principal.
    ///
    /// # Errors
    ///
    /// Returns an error without invoking Git when access is denied or identity
    /// resolution fails.
    async fn authorize(&self, request: &AuthorizationRequest) -> Result<(), AuthorizationError>;
}
