use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{MAX_REPOSITORY_LENGTH, MAX_SERVICE_LENGTH, RegistryTokenError};

/// A configured registry service, used as the exact JWT audience.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RegistryService(String);

impl RegistryService {
    /// Returns the canonical service text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for RegistryService {
    type Err = RegistryTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || value.len() > MAX_SERVICE_LENGTH
            || !value.bytes().all(is_service_character)
            || value.starts_with(['.', '-', ':'])
            || value.ends_with(['.', '-', ':'])
        {
            return Err(RegistryTokenError::InvalidService);
        }
        let mut pieces = value.split(':');
        let host = pieces.next().ok_or(RegistryTokenError::InvalidService)?;
        let port = pieces.next();
        if pieces.next().is_some()
            || host.split('.').any(str::is_empty)
            || host
                .split('.')
                .any(|label| label.starts_with(['-', '.']) || label.ends_with(['-', '.']))
            || port.is_some_and(|number| {
                number.is_empty()
                    || number.parse::<u16>().map_or(true, |parsed| parsed == 0)
                    || number
                        .parse::<u16>()
                        .is_ok_and(|parsed| parsed.to_string() != number)
            })
        {
            return Err(RegistryTokenError::InvalidService);
        }
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for RegistryService {
    type Error = RegistryTokenError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<RegistryService> for String {
    fn from(value: RegistryService) -> Self {
        value.0
    }
}

impl fmt::Display for RegistryService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A canonical OCI repository path.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepositoryName(String);

impl RepositoryName {
    /// Returns the canonical repository path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for RepositoryName {
    type Err = RegistryTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || value.len() > MAX_REPOSITORY_LENGTH
            || !value.bytes().all(is_repository_character)
            || value
                .split('/')
                .any(|component| !is_repository_component(component))
        {
            return Err(RegistryTokenError::InvalidRepository);
        }
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for RepositoryName {
    type Error = RegistryTokenError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<RepositoryName> for String {
    fn from(value: RepositoryName) -> Self {
        value.0
    }
}

impl fmt::Display for RepositoryName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One registry action accepted by this service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistryAction {
    /// Read manifests, blobs, and referrers.
    Pull,
    /// Upload blobs and publish manifests or tags.
    Push,
}

impl RegistryAction {
    pub(crate) const fn bit(self) -> u8 {
        match self {
            Self::Pull => 0b01,
            Self::Push => 0b10,
        }
    }
}

/// A set of pull and/or push actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryActions(pub(crate) u8);

impl RepositoryActions {
    /// An empty action set.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// A pull-only action set.
    #[must_use]
    pub const fn pull() -> Self {
        Self(RegistryAction::Pull.bit())
    }

    /// A push-only action set.
    #[must_use]
    pub const fn push() -> Self {
        Self(RegistryAction::Push.bit())
    }

    /// A pull-and-push action set.
    #[must_use]
    pub const fn pull_push() -> Self {
        Self(Self::pull().0 | Self::push().0)
    }

    /// Returns whether this set includes an action.
    #[must_use]
    pub const fn contains(self, action: RegistryAction) -> bool {
        self.0 & action.bit() != 0
    }

    /// Returns whether no actions are granted.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub(crate) fn actions(self) -> Vec<RegistryAction> {
        [RegistryAction::Pull, RegistryAction::Push]
            .into_iter()
            .filter(|action| self.contains(*action))
            .collect()
    }
}

/// A requested repository scope.
#[derive(Clone, PartialEq, Eq)]
pub struct RepositoryScope {
    pub(crate) repository: RepositoryName,
    pub(crate) actions: RepositoryActions,
}

impl RepositoryScope {
    /// Returns the requested repository.
    #[must_use]
    pub const fn repository(&self) -> &RepositoryName {
        &self.repository
    }

    /// Returns the requested actions.
    #[must_use]
    pub const fn actions(&self) -> RepositoryActions {
        self.actions
    }
}

const fn is_service_character(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b':')
}

const fn is_repository_character(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-' | b'/')
}

fn is_repository_component(component: &str) -> bool {
    let bytes = component.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            byte if byte.is_ascii_alphanumeric() => index += 1,
            b'-' => {
                while bytes.get(index) == Some(&b'-') {
                    index += 1;
                }
                if !bytes.get(index).is_some_and(u8::is_ascii_alphanumeric) {
                    return false;
                }
            }
            b'.' => {
                index += 1;
                if !bytes.get(index).is_some_and(u8::is_ascii_alphanumeric) {
                    return false;
                }
            }
            b'_' => {
                index += 1;
                if bytes.get(index) == Some(&b'_') {
                    index += 1;
                }
                if !bytes.get(index).is_some_and(u8::is_ascii_alphanumeric) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}
