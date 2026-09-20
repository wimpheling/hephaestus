//! Repository-owned release UI manifest parsing and validation.

mod validation;

use gateway_domain::{GatewayName, HttpMethod, RoutePath};
use release_domain::ArtifactPath;
use release_domain::ui::{
    UiIcon, UiKey, UiLabel, UiMediaType, UiPresentation, UiRoutePath, UiScope,
};
use serde::{Deserialize, Serialize};

/// Schema version for the repository's `heph.ui.toml` manifest.
pub const REPOSITORY_UIS_VERSION: u32 = 1;
/// Maximum source bytes accepted for one repository UI manifest.
pub const MAX_REPOSITORY_UIS_BYTES: usize = 256 * 1024;
/// Maximum UI declarations in one repository manifest.
pub const MAX_REPOSITORY_UIS: usize = 16;
/// Maximum API bindings on one UI declaration.
pub const MAX_UI_APIS: usize = 16;
/// Maximum static files on one UI declaration.
pub const MAX_UI_FILES: usize = 256;
/// UI kit schema version supported by this manifest.
pub const UI_KIT_VERSION: u16 = 1;

/// Repository-owned UI declarations read from `heph.ui.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryUisConfig {
    /// Manifest schema version.
    pub version: u32,
    /// Release-owned UIs declared by this repository.
    #[serde(default)]
    pub uis: Vec<RepositoryUiConfig>,
}

/// One release-owned UI declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryUiConfig {
    /// Stable UI key.
    pub key: UiKey,
    /// Installation scope.
    pub scope: UiScope,
    /// Bounded host-shell label.
    pub label: UiLabel,
    /// Allowlisted host-shell icon.
    pub icon: UiIcon,
    /// Initial host presentation.
    pub presentation: UiPresentation,
    /// Platform-relative route base.
    pub route_base: UiRoutePath,
    /// Version of the published UI kit contract.
    pub ui_kit_version: u16,
    /// Explicit cache behavior.
    pub cache: UiCachePolicy,
    /// Explicit gateway API bindings.
    #[serde(default)]
    pub apis: Vec<UiApiBinding>,
    /// Static artifact or managed service content.
    pub content: UiContent,
}

/// Cache policy for a release UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiCachePolicy {
    /// Do not permit browser or intermediary caching.
    NoStore,
}

/// One explicit gateway API binding exposed to a release UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiApiBinding {
    /// Stable UI-local API key.
    pub key: UiKey,
    /// Repository gateway declaration to invoke.
    pub gateway_name: GatewayName,
    /// Exact HTTP method.
    pub method: HttpMethod,
    /// Absolute gateway route.
    pub route: RoutePath,
}

/// Published UI content kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiContent {
    /// A release-owned static artifact tree.
    Static {
        /// Relative entrypoint route within the UI tree.
        entrypoint: UiRoutePath,
        /// Route-to-release-artifact references.
        files: Vec<UiStaticFile>,
    },
    /// A Hephaestus-managed UI service selected through a gateway route.
    ManagedService {
        /// Repository gateway declaration to invoke.
        gateway_name: GatewayName,
        /// Absolute gateway route for the service.
        route: RoutePath,
        /// Relative UI entrypoint route.
        entrypoint: UiRoutePath,
    },
}

/// One static UI route mapped to an immutable release artifact path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiStaticFile {
    /// Relative route served by the UI.
    pub route: UiRoutePath,
    /// Source release artifact path, resolved during publication later.
    pub artifact: ArtifactPath,
    /// Explicit allowlisted media type.
    pub media_type: UiMediaType,
}

/// Result of parsing one repository `heph.ui.toml` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRepositoryUis {
    /// Hash of the exact source bytes.
    pub hash: super::ConfigHash,
    /// Hash of deterministic normalized configuration serialization.
    pub normalized_hash: Option<super::ConfigHash>,
    /// Validated manifest, absent on failure.
    pub config: Option<RepositoryUisConfig>,
    /// Structured parser and validation diagnostics.
    pub diagnostics: Vec<super::Diagnostic>,
}

/// Parses and validates a repository's root-level `heph.ui.toml`.
///
/// This function validates only bounded declarative source intent. Artifact
/// identity resolution, release publication, gateway lookup, and serving are
/// separate control-plane operations.
#[must_use]
pub fn parse_repository_uis(source: &[u8]) -> ParsedRepositoryUis {
    let source_hash = super::hash(source);
    if source.len() > MAX_REPOSITORY_UIS_BYTES {
        return ParsedRepositoryUis {
            hash: source_hash,
            normalized_hash: None,
            config: None,
            diagnostics: vec![super::Diagnostic {
                code: String::from("repository_uis_too_large"),
                path: None,
                message: format!(
                    "repository UI manifest must be at most {MAX_REPOSITORY_UIS_BYTES} bytes"
                ),
            }],
        };
    }
    let text = match std::str::from_utf8(source) {
        Ok(text) => text,
        Err(error) => {
            return ParsedRepositoryUis {
                hash: source_hash,
                normalized_hash: None,
                config: None,
                diagnostics: vec![super::Diagnostic {
                    code: String::from("invalid_utf8"),
                    path: None,
                    message: error.to_string(),
                }],
            };
        }
    };
    let config = match toml::from_str::<RepositoryUisConfig>(text) {
        Ok(config) => config,
        Err(error) => {
            return ParsedRepositoryUis {
                hash: source_hash,
                normalized_hash: None,
                config: None,
                diagnostics: vec![super::Diagnostic {
                    code: String::from("invalid_toml"),
                    path: error
                        .span()
                        .map(|span| format!("bytes {}..{}", span.start, span.end)),
                    message: String::from("repository UI manifest is not valid TOML"),
                }],
            };
        }
    };
    let mut diagnostics = validation::validate(&config);
    let (normalized_hash, normalized_config) = if diagnostics.is_empty() {
        let normalized = validation::normalized(config);
        serde_json::to_vec(&normalized).map_or_else(
            |_| {
                diagnostics.push(super::Diagnostic {
                    code: String::from("normalization_failed"),
                    path: None,
                    message: String::from("validated UI manifest could not be normalized"),
                });
                (None, None)
            },
            |normalized_bytes| (Some(super::hash(&normalized_bytes)), Some(normalized)),
        )
    } else {
        (None, None)
    };
    ParsedRepositoryUis {
        hash: source_hash,
        normalized_hash,
        config: normalized_config,
        diagnostics,
    }
}
