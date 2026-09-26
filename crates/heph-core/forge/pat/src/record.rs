use super::{
    MAX_PAT_LIFETIME,
    errors::{PersonalAccessTokenAuthorizationError, PersonalAccessTokenError},
    scope::{PersonalAccessTokenMetadata, PersonalAccessTokenScope, PersonalAccessTokenStatus},
    token::{
        PersonalAccessToken, PersonalAccessTokenId, PersonalAccessTokenLabel,
        PersonalAccessTokenVerifier,
    },
};
use forge_domain::RepositoryId;
use git_capability_domain::GitOperation;
use identity_domain::{RequestId, UserId};
use time::{Duration, OffsetDateTime};

/// Safe durable PAT metadata and verifier; no bearer value is present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalAccessTokenRecord {
    id: PersonalAccessTokenId,
    verifier: PersonalAccessTokenVerifier,
    owner_user_id: UserId,
    label: PersonalAccessTokenLabel,
    scope: PersonalAccessTokenScope,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
    last_used_at: Option<OffsetDateTime>,
    creation_request_id: RequestId,
}

impl PersonalAccessTokenRecord {
    /// Creates safe durable state from a newly issued plaintext token.
    ///
    /// # Errors
    ///
    /// Returns [`PersonalAccessTokenError::InvalidLifetime`] unless expiry is
    /// strictly after creation and no more than [`MAX_PAT_LIFETIME`] later.
    pub fn issue(
        token: &PersonalAccessToken,
        owner_user_id: UserId,
        label: PersonalAccessTokenLabel,
        scope: PersonalAccessTokenScope,
        created_at: OffsetDateTime,
        expires_at: OffsetDateTime,
        creation_request_id: RequestId,
    ) -> Result<Self, PersonalAccessTokenError> {
        let lifetime = expires_at - created_at;
        if lifetime <= Duration::ZERO || lifetime > MAX_PAT_LIFETIME {
            return Err(PersonalAccessTokenError::InvalidLifetime);
        }
        Ok(Self {
            id: token.id(),
            verifier: token.verifier(),
            owner_user_id,
            label,
            scope,
            created_at,
            expires_at,
            revoked_at: None,
            last_used_at: None,
            creation_request_id,
        })
    }

    /// Restores a record from trusted durable fields while rechecking all
    /// lifecycle invariants.
    ///
    /// # Errors
    ///
    /// Returns [`PersonalAccessTokenError::InvalidLifetime`] when timestamps
    /// could not have been produced by the normal issuance and lifecycle
    /// operations.
    // Durable restoration deliberately names every persisted field so schema
    // changes cannot silently bypass domain validation.
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: PersonalAccessTokenId,
        verifier: PersonalAccessTokenVerifier,
        owner_user_id: UserId,
        label: PersonalAccessTokenLabel,
        scope: PersonalAccessTokenScope,
        created_at: OffsetDateTime,
        expires_at: OffsetDateTime,
        revoked_at: Option<OffsetDateTime>,
        last_used_at: Option<OffsetDateTime>,
        creation_request_id: RequestId,
    ) -> Result<Self, PersonalAccessTokenError> {
        let lifetime = expires_at - created_at;
        let invalid_revocation = revoked_at.is_some_and(|at| at < created_at || at >= expires_at);
        let invalid_last_use = last_used_at.is_some_and(|at| {
            at < created_at || at >= expires_at || revoked_at.is_some_and(|revoked| at > revoked)
        });
        if lifetime <= Duration::ZERO
            || lifetime > MAX_PAT_LIFETIME
            || invalid_revocation
            || invalid_last_use
        {
            return Err(PersonalAccessTokenError::InvalidLifetime);
        }
        Ok(Self {
            id,
            verifier,
            owner_user_id,
            label,
            scope,
            created_at,
            expires_at,
            revoked_at,
            last_used_at,
            creation_request_id,
        })
    }

    /// Returns the stable non-secret identifier.
    #[must_use]
    pub const fn id(&self) -> PersonalAccessTokenId {
        self.id
    }

    /// Returns the stored one-way verifier.
    #[must_use]
    pub const fn verifier(&self) -> PersonalAccessTokenVerifier {
        self.verifier
    }

    /// Returns the user who delegated authority through this PAT.
    #[must_use]
    pub const fn owner_user_id(&self) -> UserId {
        self.owner_user_id
    }

    /// Returns safe user-visible metadata.
    #[must_use]
    pub const fn label(&self) -> &PersonalAccessTokenLabel {
        &self.label
    }

    /// Returns the normalized token-level scope.
    #[must_use]
    pub const fn scope(&self) -> &PersonalAccessTokenScope {
        &self.scope
    }

    /// Returns the creation instant.
    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    /// Returns the exclusive expiry instant.
    #[must_use]
    pub const fn expires_at(&self) -> OffsetDateTime {
        self.expires_at
    }

    /// Returns the irreversible revocation instant, when present.
    #[must_use]
    pub const fn revoked_at(&self) -> Option<OffsetDateTime> {
        self.revoked_at
    }

    /// Returns the most recent successfully recorded use, when present.
    #[must_use]
    pub const fn last_used_at(&self) -> Option<OffsetDateTime> {
        self.last_used_at
    }

    /// Returns the creation request used for audit correlation.
    #[must_use]
    pub const fn creation_request_id(&self) -> RequestId {
        self.creation_request_id
    }

    /// Produces safe metadata for API or UI listing without the verifier.
    #[must_use]
    pub fn metadata(&self) -> PersonalAccessTokenMetadata {
        PersonalAccessTokenMetadata {
            id: self.id,
            owner_user_id: self.owner_user_id,
            label: self.label.clone(),
            scope: self.scope.clone(),
            created_at: self.created_at,
            expires_at: self.expires_at,
            revoked_at: self.revoked_at,
            last_used_at: self.last_used_at,
            creation_request_id: self.creation_request_id,
        }
    }

    /// Classifies lifecycle at the supplied trusted wall-clock instant.
    #[must_use]
    pub fn status_at(&self, at: OffsetDateTime) -> PersonalAccessTokenStatus {
        if at < self.created_at {
            PersonalAccessTokenStatus::NotYetValid
        } else if self.revoked_at.is_some_and(|revoked_at| at >= revoked_at) {
            PersonalAccessTokenStatus::Revoked
        } else if at >= self.expires_at {
            PersonalAccessTokenStatus::Expired
        } else {
            PersonalAccessTokenStatus::Active
        }
    }

    /// Checks verifier, owner, lifecycle, operation, and repository narrowing.
    ///
    /// The caller must still apply the owner's current repository
    /// authorization after this token-local check.
    ///
    /// # Errors
    ///
    /// Returns a safe denial reason without including bearer material.
    pub fn authorize_at(
        &self,
        token: &PersonalAccessToken,
        owner_user_id: UserId,
        operation: GitOperation,
        repository_id: RepositoryId,
        at: OffsetDateTime,
    ) -> Result<(), PersonalAccessTokenAuthorizationError> {
        if token.id() != self.id || !self.verifier.verifies(token) {
            return Err(PersonalAccessTokenAuthorizationError::InvalidCredential);
        }
        if owner_user_id != self.owner_user_id {
            return Err(PersonalAccessTokenAuthorizationError::WrongOwner);
        }
        match self.status_at(at) {
            PersonalAccessTokenStatus::Active => {}
            PersonalAccessTokenStatus::NotYetValid => {
                return Err(PersonalAccessTokenAuthorizationError::NotYetValid);
            }
            PersonalAccessTokenStatus::Revoked => {
                return Err(PersonalAccessTokenAuthorizationError::Revoked);
            }
            PersonalAccessTokenStatus::Expired => {
                return Err(PersonalAccessTokenAuthorizationError::Expired);
            }
        }
        if !self.scope.permits(operation, repository_id) {
            return Err(PersonalAccessTokenAuthorizationError::OutOfScope);
        }
        Ok(())
    }

    /// Records a successful use monotonically while the token is active.
    ///
    /// # Errors
    ///
    /// Returns an error when the instant is outside the active lifecycle or
    /// predates an already-recorded use.
    pub fn record_use(&mut self, at: OffsetDateTime) -> Result<(), PersonalAccessTokenError> {
        if self.status_at(at) != PersonalAccessTokenStatus::Active {
            return Err(PersonalAccessTokenError::InactiveToken);
        }
        if self.last_used_at.is_some_and(|previous| at < previous) {
            return Err(PersonalAccessTokenError::NonMonotonicLastUse);
        }
        self.last_used_at = Some(at);
        Ok(())
    }

    /// Irreversibly revokes an active token at the supplied instant.
    ///
    /// # Errors
    ///
    /// Returns an error when already revoked or when the instant is outside
    /// the token's active lifetime.
    pub fn revoke(&mut self, at: OffsetDateTime) -> Result<(), PersonalAccessTokenError> {
        if self.revoked_at.is_some() {
            return Err(PersonalAccessTokenError::AlreadyRevoked);
        }
        if self.status_at(at) != PersonalAccessTokenStatus::Active {
            return Err(PersonalAccessTokenError::InactiveToken);
        }
        self.revoked_at = Some(at);
        Ok(())
    }
}
