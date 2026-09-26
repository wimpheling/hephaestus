use std::collections::{BTreeMap, BTreeSet};

use crate::{
    RegistryAction, RegistryService, RegistryTokenError, RepositoryActions, RepositoryName,
    RepositoryScope,
};

/// A parsed Docker bearer-token request.
#[derive(Clone, PartialEq, Eq)]
pub struct ScopeRequest {
    pub(crate) service: RegistryService,
    pub(crate) scopes: Vec<RepositoryScope>,
}

impl ScopeRequest {
    /// Parses one service and its space-separated repository scopes.
    ///
    /// An empty scope string is valid and requests no repository access.
    /// Repeated repositories, wildcard paths, non-repository scope types, and
    /// unsupported actions are rejected rather than normalized permissively.
    ///
    /// # Errors
    ///
    /// Returns a typed error when either parameter is not canonical.
    pub fn parse(service: &str, scopes: &str) -> Result<Self, RegistryTokenError> {
        let service = service.parse()?;
        if scopes.is_empty() {
            return Ok(Self {
                service,
                scopes: Vec::new(),
            });
        }
        if !scopes.is_ascii() || scopes.split(' ').any(str::is_empty) {
            return Err(RegistryTokenError::InvalidScope);
        }

        let mut repositories = BTreeSet::new();
        let mut parsed_scopes = Vec::new();
        for scope in scopes.split(' ') {
            let parsed = parse_repository_scope(scope)?;
            if !repositories.insert(parsed.repository.clone()) {
                return Err(RegistryTokenError::DuplicateRepositoryScope);
            }
            parsed_scopes.push(parsed);
        }
        Ok(Self {
            service,
            scopes: parsed_scopes,
        })
    }

    /// Returns the requested registry service.
    #[must_use]
    pub const fn service(&self) -> &RegistryService {
        &self.service
    }

    /// Returns requested repository scopes in request order.
    #[must_use]
    pub fn scopes(&self) -> &[RepositoryScope] {
        &self.scopes
    }
}

/// The authorization result injected by the caller's live policy adapter.
#[derive(Default)]
pub struct AuthorizationDecision {
    grants: BTreeMap<RepositoryName, RepositoryActions>,
}

impl AuthorizationDecision {
    /// Creates an authorization decision with no repository grants.
    #[must_use]
    pub fn deny_all() -> Self {
        Self::default()
    }

    /// Adds or replaces the actions authorized for a repository.
    pub fn grant(&mut self, repository: RepositoryName, actions: RepositoryActions) {
        if actions.is_empty() {
            self.grants.remove(&repository);
        } else {
            self.grants.insert(repository, actions);
        }
    }

    pub(crate) fn actions_for(&self, repository: &RepositoryName) -> RepositoryActions {
        self.grants
            .get(repository)
            .copied()
            .unwrap_or_else(RepositoryActions::empty)
    }
}

fn parse_repository_scope(scope: &str) -> Result<RepositoryScope, RegistryTokenError> {
    let mut parts = scope.split(':');
    let scope_type = parts.next();
    let repository = parts.next();
    let actions = parts.next();
    if scope_type != Some("repository") || parts.next().is_some() {
        return Err(RegistryTokenError::InvalidScope);
    }
    let repository = repository
        .ok_or(RegistryTokenError::InvalidScope)?
        .parse()?;
    let actions = parse_actions(actions.ok_or(RegistryTokenError::InvalidScope)?)?;
    Ok(RepositoryScope {
        repository,
        actions,
    })
}

fn parse_actions(value: &str) -> Result<RepositoryActions, RegistryTokenError> {
    let mut actions = RepositoryActions::empty();
    for action in value.split(',') {
        let parsed = match action {
            "pull" => RegistryAction::Pull,
            "push" => RegistryAction::Push,
            _ => return Err(RegistryTokenError::InvalidActions),
        };
        if actions.contains(parsed) {
            return Err(RegistryTokenError::InvalidActions);
        }
        actions.0 |= parsed.bit();
    }
    if actions.is_empty() {
        Err(RegistryTokenError::InvalidActions)
    } else {
        Ok(actions)
    }
}
