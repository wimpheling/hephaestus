use git_capability_domain::{
    ChangedPathGlob, GitCapabilityScope, GitCapabilityScopeInput, GitOperation, RefGlob,
    RefUpdatePolicy, RepositoryId, TransferLimits,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::ReceivePolicyError;

/// Immutable authority and identity resolved for one runtime receive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRuntimeReceiveContext {
    scope: Arc<GitCapabilityScope>,
    runtime_session_id: Arc<str>,
    authorization_snapshot_id: Arc<str>,
    evaluated_at_unix_seconds: i64,
    expected_parent: Option<Arc<str>>,
}

impl ResolvedRuntimeReceiveContext {
    /// Creates a receive context from a resolved immutable capability scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the identity fields are empty, the scope is not
    /// receive-capable, or it was already expired when resolved.
    pub fn new(
        scope: Arc<GitCapabilityScope>,
        runtime_session_id: impl Into<Arc<str>>,
        authorization_snapshot_id: impl Into<Arc<str>>,
        evaluated_at_unix_seconds: i64,
    ) -> Result<Self, ReceivePolicyError> {
        let runtime_session_id = runtime_session_id.into();
        let authorization_snapshot_id = authorization_snapshot_id.into();
        if runtime_session_id.is_empty() || authorization_snapshot_id.is_empty() {
            return Err(ReceivePolicyError::InvalidResolvedContext);
        }
        if !scope.operations().contains(&GitOperation::Receive)
            || !scope.is_active_at(evaluated_at_unix_seconds)
        {
            return Err(ReceivePolicyError::InvalidResolvedContext);
        }
        Ok(Self {
            scope,
            runtime_session_id,
            authorization_snapshot_id,
            evaluated_at_unix_seconds,
            expected_parent: None,
        })
    }

    /// Creates a receive context and binds an optional exact old commit.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or non-hex expected parent, in addition
    /// to the checks performed by [`Self::new`].
    pub fn new_with_expected_parent(
        scope: Arc<GitCapabilityScope>,
        runtime_session_id: impl Into<Arc<str>>,
        authorization_snapshot_id: impl Into<Arc<str>>,
        evaluated_at_unix_seconds: i64,
        expected_parent: Option<impl Into<Arc<str>>>,
    ) -> Result<Self, ReceivePolicyError> {
        let mut context = Self::new(
            scope,
            runtime_session_id,
            authorization_snapshot_id,
            evaluated_at_unix_seconds,
        )?;
        context.expected_parent = expected_parent.map(Into::into);
        if context.expected_parent.as_deref().is_some_and(|parent| {
            !matches!(parent.len(), 40 | 64)
                || !parent
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        }) {
            return Err(ReceivePolicyError::InvalidResolvedContext);
        }
        Ok(context)
    }

    /// Returns the repository fixed by the resolved scope.
    #[must_use]
    pub fn repository_id(&self) -> RepositoryId {
        self.scope.repository_id()
    }

    /// Returns the complete immutable scope resolved by the trusted host.
    #[must_use]
    pub fn scope(&self) -> &GitCapabilityScope {
        &self.scope
    }

    /// Returns the exact runtime-session binding.
    #[must_use]
    pub fn runtime_session_id(&self) -> &str {
        &self.runtime_session_id
    }

    /// Returns the immutable authorization-snapshot binding.
    #[must_use]
    pub fn authorization_snapshot_id(&self) -> &str {
        &self.authorization_snapshot_id
    }

    /// Returns when trusted authorization state was evaluated.
    #[must_use]
    pub const fn evaluated_at_unix_seconds(&self) -> i64 {
        self.evaluated_at_unix_seconds
    }

    /// Returns the exact old commit required by this receive, when any.
    #[must_use]
    pub fn expected_parent(&self) -> Option<&str> {
        self.expected_parent.as_deref()
    }

    /// Returns whether the immutable scope remains active at `unix_seconds`.
    #[must_use]
    pub fn is_active_at(&self, unix_seconds: i64) -> bool {
        self.scope.is_active_at(unix_seconds)
    }

    /// Returns the immutable transfer ceilings.
    #[must_use]
    pub fn transfer_limits(&self) -> TransferLimits {
        self.scope.transfer_limits()
    }

    /// Serializes this trusted context for the host-owned pre-receive hook.
    ///
    /// The representation contains authority but no credential plaintext. It
    /// must be supplied only through the cleared, host-constructed backend
    /// environment, never copied from an HTTP header or request body.
    ///
    /// # Errors
    ///
    /// Returns an error if the validated context cannot be serialized.
    pub fn to_hook_json(&self) -> Result<Vec<u8>, ReceivePolicyError> {
        serde_json::to_vec(&HookContext::from(self))
            .map_err(|_| ReceivePolicyError::InvalidHookContext)
    }

    /// Parses and revalidates a host-owned pre-receive hook context.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, non-normalized, expired, or otherwise
    /// invalid authority.
    pub fn from_hook_json(bytes: &[u8]) -> Result<Self, ReceivePolicyError> {
        let wire: HookContext =
            serde_json::from_slice(bytes).map_err(|_| ReceivePolicyError::InvalidHookContext)?;
        let context: Self = wire.try_into()?;
        if context.to_hook_json()?.as_slice() != bytes {
            return Err(ReceivePolicyError::InvalidHookContext);
        }
        Ok(context)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct HookContext {
    scope: HookScope,
    runtime_session_id: String,
    authorization_snapshot_id: String,
    evaluated_at_unix_seconds: i64,
    expected_parent: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HookScope {
    version: u16,
    repository_id: RepositoryId,
    operations: Vec<GitOperation>,
    ref_globs: Vec<String>,
    changed_path_globs: Vec<String>,
    update_policy: RefUpdatePolicy,
    expires_at_unix_seconds: i64,
    transfer_limits: HookTransferLimits,
}

#[derive(Debug, Serialize, Deserialize)]
struct HookTransferLimits {
    request_bytes: u64,
    pack_bytes: u64,
    object_count: u32,
    ref_updates: u16,
}

impl From<&ResolvedRuntimeReceiveContext> for HookContext {
    fn from(context: &ResolvedRuntimeReceiveContext) -> Self {
        let scope = &context.scope;
        let limits = scope.transfer_limits();
        Self {
            scope: HookScope {
                version: scope.version(),
                repository_id: scope.repository_id(),
                operations: scope.operations().to_vec(),
                ref_globs: scope
                    .ref_globs()
                    .iter()
                    .map(|glob| glob.as_str().to_owned())
                    .collect(),
                changed_path_globs: scope
                    .changed_path_globs()
                    .iter()
                    .map(|glob| glob.as_str().to_owned())
                    .collect(),
                update_policy: scope.update_policy(),
                expires_at_unix_seconds: scope.expires_at_unix_seconds(),
                transfer_limits: HookTransferLimits {
                    request_bytes: limits.request_bytes(),
                    pack_bytes: limits.pack_bytes(),
                    object_count: limits.object_count(),
                    ref_updates: limits.ref_updates(),
                },
            },
            runtime_session_id: context.runtime_session_id().to_owned(),
            authorization_snapshot_id: context.authorization_snapshot_id().to_owned(),
            evaluated_at_unix_seconds: context.evaluated_at_unix_seconds(),
            expected_parent: context.expected_parent().map(str::to_owned),
        }
    }
}

impl TryFrom<HookContext> for ResolvedRuntimeReceiveContext {
    type Error = ReceivePolicyError;

    fn try_from(wire: HookContext) -> Result<Self, Self::Error> {
        if wire.scope.version != git_capability_domain::GRAMMAR_VERSION {
            return Err(ReceivePolicyError::InvalidHookContext);
        }
        let transfer_limits = TransferLimits::new(
            wire.scope.transfer_limits.request_bytes,
            wire.scope.transfer_limits.pack_bytes,
            wire.scope.transfer_limits.object_count,
            wire.scope.transfer_limits.ref_updates,
        )
        .map_err(|_| ReceivePolicyError::InvalidHookContext)?;
        let ref_globs = wire
            .scope
            .ref_globs
            .into_iter()
            .map(RefGlob::parse_explicitly_broad)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ReceivePolicyError::InvalidHookContext)?;
        let changed_path_globs = wire
            .scope
            .changed_path_globs
            .into_iter()
            .map(ChangedPathGlob::parse_explicitly_broad)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ReceivePolicyError::InvalidHookContext)?;
        let scope = GitCapabilityScope::new(GitCapabilityScopeInput {
            repository_id: wire.scope.repository_id,
            operations: wire.scope.operations,
            ref_globs,
            changed_path_globs,
            update_policy: wire.scope.update_policy,
            expires_at_unix_seconds: wire.scope.expires_at_unix_seconds,
            transfer_limits,
        })
        .map_err(|_| ReceivePolicyError::InvalidHookContext)?;
        Self::new_with_expected_parent(
            Arc::new(scope),
            wire.runtime_session_id,
            wire.authorization_snapshot_id,
            wire.evaluated_at_unix_seconds,
            wire.expected_parent,
        )
    }
}
