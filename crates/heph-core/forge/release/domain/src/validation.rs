use forge_domain::ProjectId;

use super::{AgentFamilyId, ReleaseValueError};

/// Validates that an update remains in the exact source family.
///
/// # Errors
///
/// Returns [`ReleaseValueError::IncompatibleAgentFamily`] when a same-named
/// export from another repository/family is supplied.
pub fn validate_update_family(
    instance_family_id: AgentFamilyId,
    candidate_family_id: AgentFamilyId,
) -> Result<(), ReleaseValueError> {
    if instance_family_id.as_uuid() == candidate_family_id.as_uuid() {
        Ok(())
    } else {
        Err(ReleaseValueError::IncompatibleAgentFamily)
    }
}

/// Validates that an attachment stays inside the consuming project.
///
/// # Errors
///
/// Returns [`ReleaseValueError::CrossProjectAttachment`] for a mismatch.
pub fn validate_attachment_project(
    instance_project_id: ProjectId,
    repository_project_id: ProjectId,
) -> Result<(), ReleaseValueError> {
    if instance_project_id.as_uuid() == repository_project_id.as_uuid() {
        Ok(())
    } else {
        Err(ReleaseValueError::CrossProjectAttachment)
    }
}
