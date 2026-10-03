use serde::Serialize;
use thiserror::Error;

/// Stable non-sensitive recipe diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecipeDiagnostic {
    /// Stable machine-readable diagnostic code.
    pub code: &'static str,
    /// Field path or named resource containing the problem.
    pub path: String,
}

/// Parsing, declaration, or authoritative resolution failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecipeError {
    /// The declaration or resolved graph violates the bounded contract.
    #[error("recipe validation failed: {0:?}")]
    Invalid(Vec<RecipeDiagnostic>),
    /// Canonical serialization unexpectedly failed.
    #[error("recipe canonical serialization failed")]
    Serialization,
}

pub fn invalid(code: &'static str, path: impl Into<String>) -> RecipeError {
    RecipeError::Invalid(vec![RecipeDiagnostic {
        code,
        path: path.into(),
    }])
}
