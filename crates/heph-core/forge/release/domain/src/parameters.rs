use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use super::{ContentHash, ParameterName};

/// Typed parameter declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterDeclaration {
    /// Stable parameter name.
    pub name: ParameterName,
    /// Parameter type and validation constraint.
    pub value_type: ParameterType,
    /// Whether consumers must explicitly or implicitly resolve a value.
    pub required: bool,
    /// Optional validated default.
    pub default: Option<ParameterValue>,
    /// Whether telemetry and UI should redact the ordinary value.
    pub sensitive: bool,
}

/// Bounded typed parameter schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParameterType {
    /// UTF-8 string with an explicit character bound.
    String {
        /// Minimum character count.
        minimum_length: u16,
        /// Maximum character count.
        maximum_length: u16,
    },
    /// Signed integer with inclusive bounds.
    Integer {
        /// Inclusive minimum.
        minimum: i64,
        /// Inclusive maximum.
        maximum: i64,
    },
    /// Boolean.
    Boolean,
    /// One of a bounded set of exact strings.
    Enum {
        /// Accepted values.
        values: Vec<String>,
    },
}

/// Typed validated parameter value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterValue {
    /// String or enum value.
    String(String),
    /// Integer value.
    Integer(i64),
    /// Boolean value.
    Boolean(bool),
}

impl ParameterType {
    /// Validates one value without coercion.
    #[must_use]
    pub fn accepts(&self, value: &ParameterValue) -> bool {
        match (self, value) {
            (
                Self::String {
                    minimum_length,
                    maximum_length,
                },
                ParameterValue::String(value),
            ) => {
                let count = value.chars().count();
                (usize::from(*minimum_length)..=usize::from(*maximum_length)).contains(&count)
            }
            (Self::Integer { minimum, maximum }, ParameterValue::Integer(value)) => {
                (*minimum..=*maximum).contains(value)
            }
            (Self::Boolean, ParameterValue::Boolean(_)) => true,
            (Self::Enum { values }, ParameterValue::String(value)) => values.contains(value),
            _ => false,
        }
    }
}

/// Canonical validated parameter document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterDocument {
    values: BTreeMap<ParameterName, ParameterValue>,
    hash: ContentHash,
}

impl ParameterDocument {
    /// Resolves provided values and defaults against the complete declaration.
    ///
    /// # Errors
    ///
    /// Returns a stable [`ParameterDiagnostic`] list for duplicate declaration
    /// names, unknown/missing parameters, or type/range mismatches.
    pub fn resolve(
        declarations: &[ParameterDeclaration],
        provided: &BTreeMap<ParameterName, ParameterValue>,
    ) -> Result<Self, Vec<ParameterDiagnostic>> {
        let mut diagnostics = Vec::new();
        let mut declarations_by_name = BTreeMap::new();
        for declaration in declarations {
            if declarations_by_name
                .insert(declaration.name.clone(), declaration)
                .is_some()
            {
                diagnostics.push(ParameterDiagnostic {
                    code: ParameterDiagnosticCode::DuplicateDeclaration,
                    parameter: Some(declaration.name.clone()),
                });
            }
            if declaration
                .default
                .as_ref()
                .is_some_and(|value| !declaration.value_type.accepts(value))
            {
                diagnostics.push(ParameterDiagnostic {
                    code: ParameterDiagnosticCode::InvalidDefault,
                    parameter: Some(declaration.name.clone()),
                });
            }
        }
        for name in provided.keys() {
            if !declarations_by_name.contains_key(name) {
                diagnostics.push(ParameterDiagnostic {
                    code: ParameterDiagnosticCode::UnknownParameter,
                    parameter: Some(name.clone()),
                });
            }
        }

        let mut values = BTreeMap::new();
        for (name, declaration) in declarations_by_name {
            let value = provided
                .get(&name)
                .cloned()
                .or_else(|| declaration.default.clone());
            match value {
                Some(value) if declaration.value_type.accepts(&value) => {
                    values.insert(name, value);
                }
                Some(_) => diagnostics.push(ParameterDiagnostic {
                    code: ParameterDiagnosticCode::InvalidValue,
                    parameter: Some(name),
                }),
                None if declaration.required => diagnostics.push(ParameterDiagnostic {
                    code: ParameterDiagnosticCode::RequiredMissing,
                    parameter: Some(name),
                }),
                None => {}
            }
        }
        if !diagnostics.is_empty() {
            diagnostics.sort_by(|left, right| {
                left.parameter
                    .cmp(&right.parameter)
                    .then_with(|| (left.code as u8).cmp(&(right.code as u8)))
            });
            return Err(diagnostics);
        }
        let hash = parameter_hash(&values);
        Ok(Self { values, hash })
    }

    /// Returns the sorted validated values.
    #[must_use]
    pub const fn values(&self) -> &BTreeMap<ParameterName, ParameterValue> {
        &self.values
    }

    /// Returns the deterministic canonical parameter hash.
    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }
}

fn update_field(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_be_bytes());
    digest.update(value);
}

fn parameter_hash(values: &BTreeMap<ParameterName, ParameterValue>) -> ContentHash {
    let mut digest = Sha256::new();
    for (name, value) in values {
        update_field(&mut digest, name.as_str().as_bytes());
        match value {
            ParameterValue::String(value) => {
                update_field(&mut digest, b"string");
                update_field(&mut digest, value.as_bytes());
            }
            ParameterValue::Integer(value) => {
                update_field(&mut digest, b"integer");
                update_field(&mut digest, &value.to_be_bytes());
            }
            ParameterValue::Boolean(value) => {
                update_field(&mut digest, b"boolean");
                update_field(&mut digest, &[u8::from(*value)]);
            }
        }
    }
    ContentHash::from_digest(digest.finalize().into())
}

/// Stable parameter compatibility diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterDiagnosticCode {
    /// Release declared the same name more than once.
    DuplicateDeclaration,
    /// A release-owned default violates its own schema.
    InvalidDefault,
    /// Consumer supplied a name the release did not declare.
    UnknownParameter,
    /// Required parameter has no supplied or default value.
    RequiredMissing,
    /// Supplied value has the wrong type or is out of bounds.
    InvalidValue,
}

/// Structured stable parameter diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterDiagnostic {
    /// Stable code.
    pub code: ParameterDiagnosticCode,
    /// Relevant parameter name.
    pub parameter: Option<ParameterName>,
}
