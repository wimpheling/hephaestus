use super::{
    MAX_REPOSITORY_RESTRICTIONS,
    errors::PersonalAccessTokenError,
    token::{PersonalAccessTokenId, PersonalAccessTokenLabel},
};
use forge_domain::RepositoryId;
use git_capability_domain::GitOperation;
use identity_domain::{RequestId, UserId};
use serde::Serialize;
use std::collections::BTreeSet;
use time::OffsetDateTime;

/// Explicit Git operations and optional exact-repository narrowing for a PAT.
///
/// An absent repository restriction means the token can participate in live
/// authorization for any repository; it never bypasses the owner's current
/// repository authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonalAccessTokenScope {
    operations: BTreeSet<GitOperation>,
    repository_restrictions: Option<BTreeSet<RepositoryId>>,
}

impl PersonalAccessTokenScope {
    /// Normalizes a non-empty operation set and optional repository set.
    ///
    /// # Errors
    ///
    /// Returns an error for no operations, an explicitly empty repository
    /// restriction, or too many restricted repositories.
    pub fn new(
        operations: impl IntoIterator<Item = GitOperation>,
        repository_restrictions: Option<impl IntoIterator<Item = RepositoryId>>,
    ) -> Result<Self, PersonalAccessTokenError> {
        let operations = operations.into_iter().collect::<BTreeSet<_>>();
        if operations.is_empty() {
            return Err(PersonalAccessTokenError::EmptyScope);
        }
        let repository_restrictions = repository_restrictions
            .map(|repositories| repositories.into_iter().collect::<BTreeSet<_>>());
        if repository_restrictions
            .as_ref()
            .is_some_and(|repositories| {
                repositories.is_empty() || repositories.len() > MAX_REPOSITORY_RESTRICTIONS
            })
        {
            return Err(PersonalAccessTokenError::InvalidRepositoryRestrictions);
        }
        Ok(Self {
            operations,
            repository_restrictions,
        })
    }

    /// Returns the normalized operation set.
    #[must_use]
    pub const fn operations(&self) -> &BTreeSet<GitOperation> {
        &self.operations
    }

    /// Returns exact repository restrictions, or `None` for no token-level
    /// narrowing beyond mandatory live user authorization.
    #[must_use]
    pub const fn repository_restrictions(&self) -> Option<&BTreeSet<RepositoryId>> {
        self.repository_restrictions.as_ref()
    }

    /// Returns whether the requested operation and repository fit this token
    /// scope. The caller must separately check current user authorization.
    #[must_use]
    pub fn permits(&self, operation: GitOperation, repository_id: RepositoryId) -> bool {
        self.operations.contains(&operation)
            && self
                .repository_restrictions
                .as_ref()
                .is_none_or(|repositories| repositories.contains(&repository_id))
    }
}

/// Lifecycle classification at one trusted wall-clock instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonalAccessTokenStatus {
    /// The supplied instant predates creation metadata.
    NotYetValid,
    /// The PAT is active, subject to scope and live user authorization.
    Active,
    /// The PAT was explicitly revoked and cannot become active again.
    Revoked,
    /// The exclusive expiry has passed.
    Expired,
}

/// Safe API/listing snapshot that deliberately excludes the verifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonalAccessTokenMetadata {
    /// Stable, non-secret token identifier.
    pub id: PersonalAccessTokenId,
    /// User who delegated authority through this PAT.
    pub owner_user_id: UserId,
    /// User-visible label.
    pub label: PersonalAccessTokenLabel,
    /// Token-level Git scope.
    pub scope: PersonalAccessTokenScope,
    /// Creation instant.
    pub created_at: OffsetDateTime,
    /// Exclusive expiry instant.
    pub expires_at: OffsetDateTime,
    /// Irreversible revocation instant, when present.
    pub revoked_at: Option<OffsetDateTime>,
    /// Most recent recorded successful use, when present.
    pub last_used_at: Option<OffsetDateTime>,
    /// Creation request used for audit correlation.
    pub creation_request_id: RequestId,
}
