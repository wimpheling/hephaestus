use serde::Serialize;

use crate::{
    GitCapabilityError,
    glob_validation::{
        glob_matches, is_broad_ref_glob, validate_concrete_path, validate_concrete_ref,
        validate_path_glob, validate_ref_glob,
    },
};

/// A strict, anchored glob over fully qualified Git refs.
///
/// Matching is case-sensitive over Unicode scalar values. Unicode
/// normalization is deliberately not performed, so canonically equivalent
/// spellings remain distinct. `*` matches within one slash-delimited segment;
/// a segment equal to `**` matches zero or more complete segments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RefGlob(String);

impl RefGlob {
    /// Parses a bounded glob and rejects a whole-namespace pattern.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, unbounded, ambiguous, or implicitly
    /// whole-namespace patterns.
    pub fn parse(value: impl Into<String>) -> Result<Self, GitCapabilityError> {
        Self::parse_inner(value.into(), false)
    }

    /// Parses a bounded glob while explicitly permitting a whole namespace.
    ///
    /// Callers should expose this separately from ordinary narrow-scope input.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern is malformed, unbounded, or
    /// ambiguous.
    pub fn parse_explicitly_broad(value: impl Into<String>) -> Result<Self, GitCapabilityError> {
        Self::parse_inner(value.into(), true)
    }

    fn parse_inner(value: String, broad: bool) -> Result<Self, GitCapabilityError> {
        validate_ref_glob(&value)?;
        if !broad && is_broad_ref_glob(&value) {
            return Err(GitCapabilityError::BroadGlobRequiresExplicitOptIn(value));
        }
        Ok(Self(value))
    }

    /// Returns the normalized glob text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether this anchored glob matches a validated ref.
    #[must_use]
    pub fn is_match(&self, reference: &str) -> bool {
        validate_concrete_ref(reference).is_ok() && glob_matches(&self.0, reference)
    }
}

/// A strict, repository-relative glob over changed Git paths.
///
/// Paths and patterns use `/` separators. Matching has the same exact Unicode,
/// case, `*`, and `**` semantics as [`RefGlob`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ChangedPathGlob(String);

impl ChangedPathGlob {
    /// Parses a bounded path glob and rejects a repository-wide pattern.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, unbounded, ambiguous, or implicitly
    /// repository-wide patterns.
    pub fn parse(value: impl Into<String>) -> Result<Self, GitCapabilityError> {
        Self::parse_inner(value.into(), false)
    }

    /// Parses a bounded path glob while explicitly permitting repository-wide
    /// matching.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern is malformed, unbounded, or
    /// ambiguous.
    pub fn parse_explicitly_broad(value: impl Into<String>) -> Result<Self, GitCapabilityError> {
        Self::parse_inner(value.into(), true)
    }

    fn parse_inner(value: String, broad: bool) -> Result<Self, GitCapabilityError> {
        validate_path_glob(&value)?;
        if !broad && matches!(value.as_str(), "*" | "**") {
            return Err(GitCapabilityError::BroadGlobRequiresExplicitOptIn(value));
        }
        Ok(Self(value))
    }

    /// Returns the normalized glob text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether this anchored glob matches a validated changed path.
    #[must_use]
    pub fn is_match(&self, path: &str) -> bool {
        validate_concrete_path(path).is_ok() && glob_matches(&self.0, path)
    }
}
