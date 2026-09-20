//! Bounded inspection of the repository-owned `heph.ui.toml` source.
//!
//! This module is intentionally kept separate from receive wiring.  It reads
//! only the exact tree entries needed to validate a UI declaration and never
//! retains the source bytes after parsing.

#[cfg(test)]
mod tests;

use agent_config::ui::{RepositoryUisConfig, UiContent};
use agent_config::{ConfigHash, Diagnostic, ParsedRepositoryGateways, RepositoryGatewaysConfig};
use forge_service::ForgeRepositoryError;

const UI_MANIFEST_PATH: &str = "heph.ui.toml";
const GATEWAY_MANIFEST_PATH: &str = "heph.gateways.toml";
const MAX_UI_SOURCE_BYTES: u64 = agent_config::ui::MAX_REPOSITORY_UIS_BYTES as u64;
const MAX_GATEWAY_SOURCE_BYTES: u64 = 1024 * 1024;
const MAX_DIAGNOSTICS: usize = 64;
const MAX_DIAGNOSTIC_BYTES: usize = 32 * 1024;

/// The source-tree entry kind observed for the UI manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UiManifestEntryKind {
    /// A regular or executable Git blob.
    Regular,
    /// A Git symbolic link.
    Symlink,
    /// A Git tree.
    Tree,
    /// A Git submodule entry.
    Gitlink,
}

/// Whether bounded source inspection accepted the declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UiManifestStatus {
    /// The UI and every required gateway declaration are valid.
    Valid,
    /// The source was present but invalid; diagnostics are redacted and bounded.
    Invalid,
}

/// Bounded UI source metadata and the validated typed representation.
///
/// Typed configs and normalized hashes are authoritative only for a fully
/// valid result.  Observed gateway OIDs, sizes, and source hashes may remain
/// available on invalid results for diagnostics.  Source bytes are
/// deliberately absent from this type.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UiManifestInspection {
    pub(super) status: UiManifestStatus,
    pub(super) requires_gateways: bool,
    pub(super) entry_kind: UiManifestEntryKind,
    pub(super) object_id: gix::ObjectId,
    pub(super) actual_size: Option<u64>,
    pub(super) source_hash: Option<ConfigHash>,
    pub(super) normalized_hash: Option<ConfigHash>,
    pub(super) config: Option<RepositoryUisConfig>,
    pub(super) gateway_object_id: Option<gix::ObjectId>,
    pub(super) gateway_actual_size: Option<u64>,
    pub(super) gateway_source_hash: Option<ConfigHash>,
    pub(super) gateway_normalized_hash: Option<ConfigHash>,
    pub(super) gateway_config: Option<RepositoryGatewaysConfig>,
    pub(super) diagnostics: Vec<Diagnostic>,
}

/// Inspects one exact commit tree for its bounded UI declaration.
///
/// A missing `heph.ui.toml` is the legacy result and returns `Ok(None)`.
/// Invalid source-tree entries and oversized manifests return a redacted
/// invalid result.  Missing or corrupt Git objects remain fatal inspection
/// errors because they indicate repository/object-store failure.
pub(super) fn inspect_repository_ui(
    repository: &gix::Repository,
    tree: &gix::Tree<'_>,
) -> Result<Option<UiManifestInspection>, ForgeRepositoryError> {
    let Some(entry) = tree
        .lookup_entry_by_path(UI_MANIFEST_PATH)
        .map_err(super::git)?
    else {
        return Ok(None);
    };

    let entry_kind = entry_kind(entry.mode())
        .ok_or_else(|| super::git("repository UI manifest has an unknown Git entry mode"))?;
    let object_id = entry.object_id();
    if entry_kind != UiManifestEntryKind::Regular {
        return Ok(Some(invalid_inspection(
            entry_kind,
            object_id,
            None,
            false,
            "invalid_repository_ui_entry",
            "repository UI manifest must be a regular Git file",
        )));
    }

    let header = repository.find_header(object_id).map_err(super::git)?;
    if header.kind() != gix::objs::Kind::Blob {
        return Err(super::git("repository UI manifest object kind is corrupt"));
    }
    let actual_size = header.size();
    if actual_size > MAX_UI_SOURCE_BYTES {
        return Ok(Some(invalid_inspection(
            entry_kind,
            object_id,
            Some(actual_size),
            false,
            "repository_ui_source_too_large",
            "repository UI manifest exceeds its source size limit",
        )));
    }

    let manifest = entry
        .object()
        .map_err(super::git)?
        .try_into_blob()
        .map_err(super::git)?;
    let parsed = agent_config::parse_repository_uis(&manifest.data);
    let agent_config::ui::ParsedRepositoryUis {
        hash: source_hash,
        normalized_hash,
        config,
        ..
    } = parsed;
    let Some(ui_config) = config else {
        return Ok(Some(
            invalid_inspection(
                entry_kind,
                object_id,
                Some(actual_size),
                false,
                "invalid_repository_ui_manifest",
                "repository UI manifest is invalid",
            )
            .with_source_hash(Some(source_hash)),
        ));
    };

    Ok(Some(inspect_valid_ui(
        repository,
        tree,
        UiManifestLocation {
            entry_kind,
            object_id,
            actual_size,
        },
        source_hash,
        normalized_hash,
        ui_config,
    )?))
}

fn inspect_valid_ui(
    repository: &gix::Repository,
    tree: &gix::Tree<'_>,
    location: UiManifestLocation,
    source_hash: ConfigHash,
    normalized_hash: Option<ConfigHash>,
    ui_config: RepositoryUisConfig,
) -> Result<UiManifestInspection, ForgeRepositoryError> {
    let requires_gateways = requires_gateway_manifest(&ui_config);
    let (gateway_source_hash, gateway_normalized_hash, gateway_config, gateway_metadata) =
        if requires_gateways {
            let gateway = inspect_required_gateways(repository, tree)?;
            if !gateway.diagnostics.is_empty() {
                let mut invalid = invalid_inspection(
                    location.entry_kind,
                    location.object_id,
                    Some(location.actual_size),
                    requires_gateways,
                    "invalid_repository_ui_gateway_manifest",
                    "required repository gateway manifest is invalid",
                )
                .with_source_hash(Some(source_hash));
                invalid.with_gateway_observation(&gateway);
                return Ok(invalid);
            }
            let Some(ref gateway_config) = gateway.config else {
                let mut invalid = invalid_inspection(
                    location.entry_kind,
                    location.object_id,
                    Some(location.actual_size),
                    requires_gateways,
                    "missing_repository_ui_gateway_manifest",
                    "required repository gateway manifest is missing",
                )
                .with_source_hash(Some(source_hash));
                invalid.with_gateway_observation(&gateway);
                return Ok(invalid);
            };
            let cross_manifest_diagnostics = agent_config::validate_repository_uis_against_gateways(
                &ui_config,
                Some(gateway_config),
            );
            if !cross_manifest_diagnostics.is_empty() {
                let mut invalid = invalid_inspection(
                    location.entry_kind,
                    location.object_id,
                    Some(location.actual_size),
                    requires_gateways,
                    "invalid_repository_ui_gateway_reference",
                    "repository UI gateway references are invalid",
                )
                .with_source_hash(Some(source_hash));
                invalid.with_gateway_observation(&gateway);
                return Ok(invalid);
            }
            (
                gateway.source_hash,
                gateway.normalized_hash,
                Some(gateway_config.clone()),
                (gateway.object_id, gateway.actual_size),
            )
        } else {
            (None, None, None, (None, None))
        };

    Ok(UiManifestInspection {
        status: UiManifestStatus::Valid,
        requires_gateways,
        entry_kind: location.entry_kind,
        object_id: location.object_id,
        actual_size: Some(location.actual_size),
        source_hash: Some(source_hash),
        normalized_hash,
        config: Some(ui_config),
        gateway_object_id: gateway_metadata.0,
        gateway_actual_size: gateway_metadata.1,
        gateway_source_hash,
        gateway_normalized_hash,
        gateway_config,
        diagnostics: Vec::new(),
    })
}

#[derive(Debug, Clone, Copy)]
struct UiManifestLocation {
    entry_kind: UiManifestEntryKind,
    object_id: gix::ObjectId,
    actual_size: u64,
}

fn inspect_required_gateways(
    repository: &gix::Repository,
    tree: &gix::Tree<'_>,
) -> Result<GatewayManifestInspection, ForgeRepositoryError> {
    let Some(entry) = tree
        .lookup_entry_by_path(GATEWAY_MANIFEST_PATH)
        .map_err(super::git)?
    else {
        return Ok(GatewayManifestInspection {
            object_id: None,
            actual_size: None,
            source_hash: None,
            normalized_hash: None,
            config: None,
            diagnostics: vec![diagnostic(
                "missing_repository_ui_gateway_manifest",
                "required repository gateway manifest is missing",
            )],
        });
    };
    let kind = entry_kind(entry.mode())
        .ok_or_else(|| super::git("repository gateway manifest has an unknown Git entry mode"))?;
    let object_id = entry.object_id();
    if kind != UiManifestEntryKind::Regular {
        return Ok(GatewayManifestInspection {
            object_id: Some(object_id),
            actual_size: None,
            source_hash: None,
            normalized_hash: None,
            config: None,
            diagnostics: vec![diagnostic(
                "invalid_repository_ui_gateway_manifest",
                "required repository gateway manifest must be a regular Git file",
            )],
        });
    }
    let header = repository.find_header(object_id).map_err(super::git)?;
    if header.kind() != gix::objs::Kind::Blob {
        return Err(super::git(
            "repository gateway manifest object kind is corrupt",
        ));
    }
    if header.size() > MAX_GATEWAY_SOURCE_BYTES {
        return Ok(GatewayManifestInspection {
            object_id: Some(object_id),
            actual_size: Some(header.size()),
            source_hash: None,
            normalized_hash: None,
            config: None,
            diagnostics: vec![diagnostic(
                "repository_ui_gateway_source_too_large",
                "required repository gateway manifest exceeds its source size limit",
            )],
        });
    }
    let manifest = entry
        .object()
        .map_err(super::git)?
        .try_into_blob()
        .map_err(super::git)?;
    let parsed = agent_config::parse_repository_gateways(&manifest.data);
    let ParsedRepositoryGateways {
        hash,
        normalized_hash,
        config,
        diagnostics,
    } = parsed;
    let Some(config) = config else {
        return Ok(GatewayManifestInspection {
            object_id: Some(object_id),
            actual_size: Some(header.size()),
            source_hash: Some(hash),
            normalized_hash: None,
            config: None,
            diagnostics: redact_diagnostics(&diagnostics, "invalid_repository_ui_gateway_manifest"),
        });
    };
    let config = agent_config::canonical_repository_gateways(&config);
    Ok(GatewayManifestInspection {
        object_id: Some(object_id),
        actual_size: Some(header.size()),
        source_hash: Some(hash),
        normalized_hash,
        config: Some(config),
        diagnostics: Vec::new(),
    })
}

#[derive(Debug, Clone, PartialEq)]
struct GatewayManifestInspection {
    object_id: Option<gix::ObjectId>,
    actual_size: Option<u64>,
    source_hash: Option<ConfigHash>,
    normalized_hash: Option<ConfigHash>,
    config: Option<RepositoryGatewaysConfig>,
    diagnostics: Vec<Diagnostic>,
}

fn requires_gateway_manifest(config: &RepositoryUisConfig) -> bool {
    config
        .uis
        .iter()
        .any(|ui| !ui.apis.is_empty() || matches!(ui.content, UiContent::ManagedService { .. }))
}

const fn entry_kind(mode: gix::objs::tree::EntryMode) -> Option<UiManifestEntryKind> {
    if mode.is_blob() {
        Some(UiManifestEntryKind::Regular)
    } else if mode.is_link() {
        Some(UiManifestEntryKind::Symlink)
    } else if mode.is_tree() {
        Some(UiManifestEntryKind::Tree)
    } else if mode.is_commit() {
        Some(UiManifestEntryKind::Gitlink)
    } else {
        None
    }
}

fn invalid_inspection(
    entry_kind: UiManifestEntryKind,
    object_id: gix::ObjectId,
    actual_size: Option<u64>,
    requires_gateways: bool,
    code: &'static str,
    message: &'static str,
) -> UiManifestInspection {
    UiManifestInspection {
        status: UiManifestStatus::Invalid,
        requires_gateways,
        entry_kind,
        object_id,
        actual_size,
        source_hash: None,
        normalized_hash: None,
        config: None,
        gateway_object_id: None,
        gateway_actual_size: None,
        gateway_source_hash: None,
        gateway_normalized_hash: None,
        gateway_config: None,
        diagnostics: vec![diagnostic(code, message)],
    }
}

fn diagnostic(code: &'static str, message: &'static str) -> Diagnostic {
    Diagnostic {
        code: code.to_owned(),
        path: Some(String::from("manifest")),
        message: message.to_owned(),
    }
}

fn redact_diagnostics(diagnostics: &[Diagnostic], fallback_code: &'static str) -> Vec<Diagnostic> {
    let mut result = Vec::new();
    let mut used = 0usize;
    for diagnostic in diagnostics.iter().take(MAX_DIAGNOSTICS) {
        let code = stable_code(&diagnostic.code).unwrap_or_else(|| fallback_code.to_owned());
        let value = Diagnostic {
            code,
            path: Some(String::from("manifest")),
            message: String::from("repository manifest is invalid"),
        };
        let bytes = value.code.len() + value.message.len() + 8;
        if used.saturating_add(bytes) > MAX_DIAGNOSTIC_BYTES {
            break;
        }
        used += bytes;
        result.push(value);
    }
    if result.is_empty() && !diagnostics.is_empty() {
        result.push(diagnostic(fallback_code, "repository manifest is invalid"));
    }
    result
}

fn stable_code(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
    .then(|| value.to_owned())
}

impl UiManifestInspection {
    fn with_source_hash(mut self, source_hash: Option<ConfigHash>) -> Self {
        self.source_hash = source_hash;
        self.diagnostics = redact_diagnostics(&self.diagnostics, "invalid_repository_ui_manifest");
        self
    }

    fn with_gateway_observation(&mut self, gateway: &GatewayManifestInspection) {
        self.gateway_object_id = gateway.object_id;
        self.gateway_actual_size = gateway.actual_size;
        self.gateway_source_hash.clone_from(&gateway.source_hash);
        self.gateway_normalized_hash = None;
        self.gateway_config = None;
    }
}
