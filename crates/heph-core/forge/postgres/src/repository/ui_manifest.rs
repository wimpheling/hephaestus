//! Bounded inspection of the repository-owned `heph.ui.toml` source.
//!
//! This module is intentionally kept separate from receive wiring.  It reads
//! only the exact tree entries needed to validate a UI declaration and never
//! retains the source bytes after parsing.

#[path = "ui_manifest/inspection.rs"]
mod inspection;
#[path = "ui_manifest/types.rs"]
mod types;
#[path = "ui_manifest/validation.rs"]
mod validation;

use agent_config::ui::MAX_REPOSITORY_UIS_BYTES;

const UI_MANIFEST_PATH: &str = "heph.ui.toml";
const GATEWAY_MANIFEST_PATH: &str = "heph.gateways.toml";
const MAX_UI_SOURCE_BYTES: u64 = MAX_REPOSITORY_UIS_BYTES as u64;
const MAX_GATEWAY_SOURCE_BYTES: u64 = 1024 * 1024;
const MAX_DIAGNOSTICS: usize = 64;
const MAX_DIAGNOSTIC_BYTES: usize = 32 * 1024;

pub(super) use inspection::inspect_repository_ui;
pub(super) use types::{UiManifestEntryKind, UiManifestInspection, UiManifestStatus};

#[cfg(test)]
mod tests;
