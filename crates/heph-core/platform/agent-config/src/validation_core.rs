//! Top-level agent configuration validation.
use super::{
    validation_images::validate_image_selection,
    validation_shared::{diagnostic, validate_absolute_path},
    validation_v2::validate_v2,
};
use crate::{AgentConfig, Diagnostic, PublicationMode, REUSABLE_RELEASE_VERSION};
use capability_domain::CapabilityResourceKind;
use forge_domain::GitRef;
use std::{collections::HashSet, path::Path};

// Keeping ordered checks together preserves diagnostic order.
#[allow(clippy::too_many_lines)]
pub fn validate(config: &AgentConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if config.version != REUSABLE_RELEASE_VERSION {
        diagnostic(
            &mut diagnostics,
            "unsupported_version",
            "version",
            format!(
                "configuration version {} is unsupported; expected {REUSABLE_RELEASE_VERSION}",
                config.version
            ),
        );
        return diagnostics;
    }
    if config.agent.name.trim().is_empty() || config.agent.name.len() > 128 {
        diagnostic(
            &mut diagnostics,
            "invalid_agent_name",
            "agent.name",
            "name must contain 1 to 128 characters",
        );
    }
    validate_v2(config, &mut diagnostics);
    if !(1..=64).contains(&config.resources.vcpus) {
        diagnostic(
            &mut diagnostics,
            "invalid_vcpus",
            "resources.vcpus",
            "vcpus must be between 1 and 64",
        );
    }
    if !(128..=1_048_576).contains(&config.resources.memory_mib) {
        diagnostic(
            &mut diagnostics,
            "invalid_memory",
            "resources.memory_mib",
            "memory_mib must be between 128 and 1048576",
        );
    }
    validate_image_selection(&mut diagnostics, "guest.image", &config.guest.image, false);
    if config.workspace.mount {
        validate_absolute_path(
            &mut diagnostics,
            "workspace.path",
            &config.workspace.path,
            "invalid_workspace_path",
        );
    }
    if config.publication.mode == PublicationMode::Proposal
        && config.workspace.mount
        && !config.workspace.read_only
    {
        diagnostic(
            &mut diagnostics,
            "proposal_workspace_must_be_read_only",
            "workspace.read_only",
            "proposal mode exposes only a read-only source tree and never a Git write remote",
        );
    }
    if config.publication.mode == PublicationMode::RuntimeGit && config.workspace.mount {
        diagnostic(
            &mut diagnostics,
            "runtime_git_uses_capability_worktrees",
            "workspace.mount",
            "runtime_git mode cannot infer repository access from the legacy proposal workspace",
        );
    }
    match (
        config.publication.mode,
        config.publication.repository_slot.as_ref(),
    ) {
        (PublicationMode::Proposal, Some(_)) => diagnostic(
            &mut diagnostics,
            "proposal_repository_slot_forbidden",
            "publication.repository_slot",
            "proposal mode does not select a runtime repository capability",
        ),
        (PublicationMode::RuntimeGit, None) => diagnostic(
            &mut diagnostics,
            "runtime_git_repository_slot_required",
            "publication.repository_slot",
            "runtime_git mode requires an explicit repository capability slot",
        ),
        (PublicationMode::RuntimeGit, Some(slot)) => {
            let declaration = config
                .capability_slots
                .iter()
                .find(|declaration| declaration.key == slot.as_str());
            if declaration.is_none_or(|declaration| {
                declaration.resource_kind != CapabilityResourceKind::Repository
                    || !declaration.required
                    || declaration.git.is_none()
            }) {
                diagnostic(
                    &mut diagnostics,
                    "runtime_git_repository_slot_invalid",
                    "publication.repository_slot",
                    "runtime_git repository_slot must name a required repository capability slot with a typed Git ceiling",
                );
            }
        }
        (PublicationMode::Proposal, None) => {}
    }
    if config.results.declared_files.len() > 128 {
        diagnostic(
            &mut diagnostics,
            "too_many_declared_files",
            "results.declared_files",
            "at most 128 result files may be declared",
        );
    }
    let mut declared_files = HashSet::new();
    for (index, result_path) in config.results.declared_files.iter().enumerate() {
        let path = Path::new(result_path);
        let valid = !path.is_absolute()
            && path.components().next().is_some()
            && path.components().all(|component| {
                matches!(component, std::path::Component::Normal(value)
                    if !value.eq_ignore_ascii_case(std::ffi::OsStr::new(".git")))
            });
        if !valid {
            diagnostic(
                &mut diagnostics,
                "invalid_declared_file",
                format!("results.declared_files[{index}]"),
                "declared file paths must be relative, traversal-free, and outside .git",
            );
        } else if !declared_files.insert(result_path) {
            diagnostic(
                &mut diagnostics,
                "duplicate_declared_file",
                format!("results.declared_files[{index}]"),
                "declared file paths must be unique",
            );
        }
    }
    for (index, pattern) in config.triggers.refs.iter().enumerate() {
        let value = pattern.strip_suffix("/*").unwrap_or(pattern);
        if GitRef::parse(value.to_owned()).is_err() {
            diagnostic(
                &mut diagnostics,
                "invalid_trigger_ref",
                format!("triggers.refs[{index}]"),
                "trigger must be a fully-qualified ref or end in /*",
            );
        }
    }
    diagnostics
}
