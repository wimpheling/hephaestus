use release_domain::{ParameterDeclaration, ParameterName, ParameterType, ParameterValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::{MAX_VALUE_BYTES, RecipeError, errors::invalid};

/// Strict recipe-owned ordinary input syntax.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDeclaration {
    /// Unique bounded input name.
    pub name: ParameterName,
    /// Explicit type and limits.
    pub value_type: InputType,
    /// Whether supplied value or default must resolve.
    #[serde(default)]
    pub required: bool,
    /// Optional ordinary default; secrets are unsupported.
    #[serde(default)]
    pub default: Option<ParameterValue>,
}

/// Strict input schema mirroring reusable release parameter semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputType {
    /// Explicit character length bounds, capped by recipe limits.
    String {
        /// Inclusive minimum character count.
        minimum_length: u16,
        /// Inclusive maximum character count.
        maximum_length: u16,
    },
    /// Explicit signed integer bounds.
    Integer {
        /// Inclusive minimum.
        minimum: i64,
        /// Inclusive maximum.
        maximum: i64,
    },
    /// Boolean without coercion.
    // Empty struct variants ensure Serde rejects unknown nested fields.
    Boolean {},
    /// Bounded nonempty unique choices.
    Enum {
        /// Exact accepted values.
        values: Vec<String>,
    },
}

impl InputDeclaration {
    pub(super) fn declaration(&self) -> ParameterDeclaration {
        ParameterDeclaration {
            name: self.name.clone(),
            value_type: self.value_type.parameter_type(),
            required: self.required,
            default: self.default.clone(),
            sensitive: false,
        }
    }

    pub(super) fn validate(&self) -> Result<(), RecipeError> {
        if !self.value_type.well_formed() {
            return Err(invalid("invalid_input_bounds", self.name.as_str()));
        }
        if self.default.as_ref().is_some_and(|value| {
            !bounded_value(value) || !self.value_type.parameter_type().accepts(value)
        }) {
            return Err(invalid("invalid_input_default", self.name.as_str()));
        }
        Ok(())
    }
}

impl InputType {
    pub(super) fn parameter_type(&self) -> ParameterType {
        match self {
            Self::String {
                minimum_length,
                maximum_length,
            } => ParameterType::String {
                minimum_length: *minimum_length,
                maximum_length: *maximum_length,
            },
            Self::Integer { minimum, maximum } => ParameterType::Integer {
                minimum: *minimum,
                maximum: *maximum,
            },
            Self::Boolean {} => ParameterType::Boolean,
            Self::Enum { values } => ParameterType::Enum {
                values: values.clone(),
            },
        }
    }

    pub(super) fn well_formed(&self) -> bool {
        match self {
            Self::String {
                minimum_length,
                maximum_length,
            } => {
                minimum_length <= maximum_length && usize::from(*maximum_length) <= MAX_VALUE_BYTES
            }
            Self::Integer { minimum, maximum } => minimum <= maximum,
            Self::Boolean {} => true,
            Self::Enum { values } => {
                !values.is_empty()
                    && values.len() <= 64
                    && values
                        .iter()
                        .all(|value| !value.is_empty() && value.len() <= 256)
                    && values.iter().collect::<BTreeSet<_>>().len() == values.len()
            }
        }
    }
}

pub fn valid_parameter(declaration: &ParameterDeclaration) -> bool {
    let schema = match &declaration.value_type {
        ParameterType::String {
            minimum_length,
            maximum_length,
        } => InputType::String {
            minimum_length: *minimum_length,
            maximum_length: *maximum_length,
        },
        ParameterType::Integer { minimum, maximum } => InputType::Integer {
            minimum: *minimum,
            maximum: *maximum,
        },
        ParameterType::Boolean => InputType::Boolean {},
        ParameterType::Enum { values } => InputType::Enum {
            values: values.clone(),
        },
    };
    !declaration.sensitive
        && schema.well_formed()
        && declaration
            .default
            .as_ref()
            .is_none_or(|value| bounded_value(value) && declaration.value_type.accepts(value))
}

pub const fn bounded_value(value: &ParameterValue) -> bool {
    !matches!(value, ParameterValue::String(value) if value.len() > MAX_VALUE_BYTES)
}
