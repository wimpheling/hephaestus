//! Parameter and secret slot validation.
use super::validation_shared::{diagnostic, valid_key};
use crate::{
    Diagnostic, ParameterDeclaration, ParameterDefault, ParameterType, SecretSlotDeclaration,
};
use std::collections::HashSet;

pub fn validate_parameters(parameters: &[ParameterDeclaration], diagnostics: &mut Vec<Diagnostic>) {
    let mut names = HashSet::new();
    for (index, parameter) in parameters.iter().enumerate() {
        if !valid_key(&parameter.name, 64) || parameter.name.starts_with("hephaestus_") {
            diagnostic(
                diagnostics,
                "invalid_parameter_name",
                format!("parameters[{index}].name"),
                "parameter name is malformed or reserved",
            );
        } else if !names.insert(&parameter.name) {
            diagnostic(
                diagnostics,
                "duplicate_parameter",
                format!("parameters[{index}].name"),
                "parameter names must be unique",
            );
        }
        let schema_valid = match &parameter.value_type {
            ParameterType::String {
                minimum_length,
                maximum_length,
            } => minimum_length <= maximum_length && *maximum_length <= 4096,
            ParameterType::Integer { minimum, maximum } => minimum <= maximum,
            ParameterType::Boolean => true,
            ParameterType::Enum { values } => {
                (1..=64).contains(&values.len())
                    && values.iter().all(|value| (1..=128).contains(&value.len()))
                    && values.iter().collect::<HashSet<_>>().len() == values.len()
            }
        };
        if !schema_valid {
            diagnostic(
                diagnostics,
                "invalid_parameter_schema",
                format!("parameters[{index}]"),
                "parameter type must have explicit valid bounds and unique choices",
            );
        }
        if parameter.required && parameter.default.is_none() {
            continue;
        }
        if let Some(default) = &parameter.default
            && !parameter_default_matches(&parameter.value_type, default)
        {
            diagnostic(
                diagnostics,
                "invalid_parameter_default",
                format!("parameters[{index}].default"),
                "parameter default does not match its declared type and bounds",
            );
        }
    }
}

pub fn parameter_default_matches(value_type: &ParameterType, value: &ParameterDefault) -> bool {
    match (value_type, value) {
        (
            ParameterType::String {
                minimum_length,
                maximum_length,
            },
            ParameterDefault::String(value),
        ) => (usize::from(*minimum_length)..=usize::from(*maximum_length))
            .contains(&value.chars().count()),
        (ParameterType::Integer { minimum, maximum }, ParameterDefault::Integer(value)) => {
            (*minimum..=*maximum).contains(value)
        }
        (ParameterType::Boolean, ParameterDefault::Boolean(_)) => true,
        (ParameterType::Enum { values }, ParameterDefault::String(value)) => values.contains(value),
        _ => false,
    }
}

pub fn validate_secret_slots(slots: &[SecretSlotDeclaration], diagnostics: &mut Vec<Diagnostic>) {
    let mut keys = HashSet::new();
    for (index, slot) in slots.iter().enumerate() {
        if !valid_key(&slot.key, 64) {
            diagnostic(
                diagnostics,
                "invalid_secret_slot_key",
                format!("secret_slots[{index}].key"),
                "secret slot key must be a bounded lowercase identifier",
            );
        } else if !keys.insert(&slot.key) {
            diagnostic(
                diagnostics,
                "duplicate_secret_slot",
                format!("secret_slots[{index}].key"),
                "secret slot keys must be unique",
            );
        }
        if slot.purpose.trim().is_empty() || slot.purpose.len() > 512 {
            diagnostic(
                diagnostics,
                "invalid_secret_slot_purpose",
                format!("secret_slots[{index}].purpose"),
                "secret slot purpose must contain 1 to 512 characters",
            );
        }
        if slot.delivery_modes.is_empty()
            || slot.delivery_modes.len() > 2
            || slot.delivery_modes.iter().collect::<HashSet<_>>().len() != slot.delivery_modes.len()
            || slot.phases.is_empty()
            || slot.phases.len() > 2
            || slot.phases.iter().collect::<HashSet<_>>().len() != slot.phases.len()
            || slot.destinations.len() > 32
        {
            diagnostic(
                diagnostics,
                "invalid_secret_slot_policy",
                format!("secret_slots[{index}]"),
                "secret slot modes, phases, or destinations are empty, duplicated, or oversized",
            );
        }
    }
}
