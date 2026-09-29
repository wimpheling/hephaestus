use std::{fs, path::Path, process::Command};

use super::{ArchitectureConfiguration, CONFIGURATION, CargoMetadata, Diagnostic};
use crate::process::{DevError, Result};

pub(super) fn read_configuration(root: &Path) -> Result<ArchitectureConfiguration> {
    let path = root.join(CONFIGURATION);
    let source = fs::read_to_string(&path).map_err(|error| {
        DevError::Invalid(
            Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!("cannot read {}: {error}", path.display()),
            )
            .render(),
        )
    })?;
    toml::from_str(&source).map_err(|error| {
        DevError::Invalid(
            Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!("cannot parse {}: {error}", path.display()),
            )
            .render(),
        )
    })
}

pub(super) fn cargo_metadata(root: &Path) -> Result<CargoMetadata> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(DevError::Invalid(
            Diagnostic::new(
                "ARCH-CARGO-METADATA",
                format!(
                    "cargo metadata failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            )
            .render(),
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        DevError::Invalid(
            Diagnostic::new(
                "ARCH-CARGO-METADATA",
                format!("cargo metadata returned invalid JSON: {error}"),
            )
            .render(),
        )
    })
}
