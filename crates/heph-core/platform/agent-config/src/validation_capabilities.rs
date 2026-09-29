//! Capability slot validation diagnostics.
use super::validation_shared::diagnostic;
use crate::{CapabilitySlotDeclaration, Diagnostic};
use capability_domain::{
    CapabilityError, CapabilityOperation, CapabilityRequirementId, CapabilityResourceKind,
};
use std::collections::HashSet;

pub fn validate_capability_slots(
    slots: &[CapabilitySlotDeclaration],
    diagnostics: &mut Vec<Diagnostic>,
) {
    if slots.len() > 64 {
        diagnostic(
            diagnostics,
            "too_many_capability_slots",
            "capability_slots",
            "a release agent may declare at most 64 capability slots",
        );
    }

    let mut keys = HashSet::new();
    for (index, slot) in slots.iter().enumerate() {
        if !keys.insert(slot.key.as_str()) {
            diagnostic(
                diagnostics,
                "duplicate_capability_slot",
                format!("capability_slots[{index}].key"),
                "capability slot keys must be unique",
            );
        }
        if slot.purpose.trim().is_empty() || slot.purpose.len() > 512 {
            diagnostic(
                diagnostics,
                "invalid_capability_slot_purpose",
                format!("capability_slots[{index}].purpose"),
                "capability slot purpose must contain 1 to 512 characters",
            );
        }

        if let Err(error) =
            slot.to_requirement(CapabilityRequirementId::from_uuid(uuid::Uuid::nil()))
        {
            capability_diagnostic(diagnostics, index, slot, &error);
        }
        if slot.git.is_some() && slot.resource_kind != CapabilityResourceKind::Repository {
            diagnostic(
                diagnostics,
                "git_scope_requires_repository",
                format!("capability_slots[{index}].git"),
                "a typed Git ceiling is valid only on a repository capability slot",
            );
        } else if slot.git_ceiling().is_err() {
            diagnostic(
                diagnostics,
                "invalid_git_capability_ceiling",
                format!("capability_slots[{index}].git"),
                "Git patterns, transition rules, operations, and transfer limits must form a bounded normalized ceiling",
            );
        }
    }
}

pub fn capability_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    index: usize,
    slot: &CapabilitySlotDeclaration,
    error: &CapabilityError,
) {
    let base = format!("capability_slots[{index}]");
    let (code, path) = match *error {
        CapabilityError::InvalidSlotKey => ("invalid_capability_slot_key", format!("{base}.key")),
        CapabilityError::DuplicateOperation(operation) => {
            let field = if operation_occurrences(&slot.required_operations, operation) > 1 {
                "required_operations"
            } else {
                "optional_operations"
            };
            ("duplicate_capability_operation", format!("{base}.{field}"))
        }
        CapabilityError::EmptyOperationSet => ("empty_capability_operations", base),
        CapabilityError::TooManyOperations => ("too_many_capability_operations", base),
        CapabilityError::IllegalOperation { operation, .. } => {
            let field = if slot.required_operations.contains(&operation) {
                "required_operations"
            } else {
                "optional_operations"
            };
            ("illegal_capability_operation", format!("{base}.{field}"))
        }
        CapabilityError::OperationRequiredAndOptional(_) => {
            ("overlapping_capability_operations", base)
        }
        _ => ("invalid_capability_slot", base),
    };
    diagnostic(diagnostics, code, path, error.to_string());
}

pub fn operation_occurrences(
    operations: &[CapabilityOperation],
    expected: CapabilityOperation,
) -> usize {
    operations
        .iter()
        .filter(|operation| **operation == expected)
        .count()
}
