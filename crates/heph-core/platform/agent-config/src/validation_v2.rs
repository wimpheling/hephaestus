//! Version 2 agent validation.
use super::{
    validation_capabilities::validate_capability_slots,
    validation_images::validate_image_selection,
    validation_parameters::{validate_parameters, validate_secret_slots},
    validation_shared::{
        diagnostic, valid_key, valid_relative_path, validate_absolute_path, validate_relative_path,
    },
};
use crate::{AgentConfig, Diagnostic};
use forge_domain::GitRef;
use std::collections::HashSet;

// Keep the versioned schema checks together so diagnostics retain their stable order.
#[allow(clippy::too_many_lines)]
pub fn validate_v2(config: &AgentConfig, diagnostics: &mut Vec<Diagnostic>) {
    if config
        .agent
        .key
        .as_deref()
        .is_none_or(|key| !valid_key(key, 64))
    {
        diagnostic(
            diagnostics,
            "invalid_agent_key",
            "agent.key",
            "version 2 requires a stable lowercase key of at most 64 characters",
        );
    }
    validate_relative_path(
        diagnostics,
        "guest.command",
        &config.guest.command,
        "invalid_release_command",
    );
    validate_relative_path(
        diagnostics,
        "guest.working_directory",
        &config.guest.working_directory,
        "invalid_release_working_directory",
    );
    validate_image_selection(diagnostics, "guest.image", &config.guest.image, false);
    let Some(build) = &config.build else {
        diagnostic(
            diagnostics,
            "missing_build",
            "build",
            "version 2 requires an isolated build definition",
        );
        return;
    };
    validate_absolute_path(
        diagnostics,
        "build.command",
        &build.command,
        "invalid_build_command",
    );
    validate_absolute_path(
        diagnostics,
        "build.working_directory",
        &build.working_directory,
        "invalid_build_working_directory",
    );
    validate_image_selection(diagnostics, "build.image", &build.image, true);
    if build.artifacts.is_empty() || build.artifacts.len() > 128 {
        diagnostic(
            diagnostics,
            "invalid_build_artifact_count",
            "build.artifacts",
            "build must declare between 1 and 128 outputs",
        );
    }
    let mut paths = HashSet::new();
    for (index, artifact) in build.artifacts.iter().enumerate() {
        if !valid_relative_path(&artifact.path) {
            diagnostic(
                diagnostics,
                "invalid_build_artifact_path",
                format!("build.artifacts[{index}].path"),
                "artifact path must be relative, traversal-free, and outside .git",
            );
        } else if !paths.insert(&artifact.path) {
            diagnostic(
                diagnostics,
                "duplicate_build_artifact_path",
                format!("build.artifacts[{index}].path"),
                "artifact paths must be unique",
            );
        }
    }
    for (index, trigger) in build.triggers.iter().enumerate() {
        let value = trigger.strip_suffix("/*").unwrap_or(trigger);
        if GitRef::parse(value.to_owned()).is_err() {
            diagnostic(
                diagnostics,
                "invalid_build_trigger",
                format!("build.triggers[{index}]"),
                "build trigger must be an exact ref or terminal prefix",
            );
        }
    }
    validate_parameters(&config.parameters, diagnostics);
    validate_secret_slots(&config.secret_slots, diagnostics);
    validate_capability_slots(&config.capability_slots, diagnostics);
    if let Some(hook) = &config.update_hook {
        validate_relative_path(
            diagnostics,
            "update_hook.command",
            &hook.command,
            "invalid_update_hook_command",
        );
        if !(1..=86_400).contains(&hook.timeout_seconds) {
            diagnostic(
                diagnostics,
                "invalid_update_hook_timeout",
                "update_hook.timeout_seconds",
                "update hook timeout must be between 1 and 86400 seconds",
            );
        }
    }
}
