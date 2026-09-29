//! TOML parsing and normalization entry points.
use super::{
    validation_core::validate,
    validation_images::{validate_repository_gateways, validate_repository_oci_images},
};
use crate::{
    AgentConfig, ConfigHash, Diagnostic, ParsedConfig, ParsedRepositoryGateways,
    ParsedRepositoryOciImages, RepositoryGatewaysConfig, RepositoryOciImagesConfig,
};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Parses and validates `agent.toml`.
#[must_use]
pub fn parse(source: &[u8]) -> ParsedConfig {
    let source_hash = hash(source);
    let text = match std::str::from_utf8(source) {
        Ok(text) => text,
        Err(error) => {
            return ParsedConfig {
                hash: source_hash,
                normalized_hash: None,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_utf8"),
                    path: None,
                    message: error.to_string(),
                }],
            };
        }
    };
    let config = match toml::from_str::<AgentConfig>(text) {
        Ok(config) => config,
        Err(error) => {
            return ParsedConfig {
                hash: source_hash,
                normalized_hash: None,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_toml"),
                    path: error
                        .span()
                        .map(|span| format!("bytes {}..{}", span.start, span.end)),
                    message: error.message().to_owned(),
                }],
            };
        }
    };
    let mut diagnostics = validate(&config);
    let normalized_hash = if diagnostics.is_empty() {
        let normalized = normalized_config(config.clone());
        toml::to_string(&normalized).map_or_else(
            |_| {
                diagnostics.push(Diagnostic {
                    code: String::from("normalization_failed"),
                    path: None,
                    message: String::from("validated configuration could not be normalized"),
                });
                None
            },
            |normalized| Some(hash(normalized.as_bytes())),
        )
    } else {
        None
    };
    ParsedConfig {
        hash: source_hash,
        normalized_hash,
        config: diagnostics.is_empty().then_some(config),
        diagnostics,
    }
}

/// Parses and validates a repository's root-level `heph.images.toml`.
///
/// This function only validates the declarative source contract. The receive
/// workflow verifies the declared paths against the exact Git tree and resolves
/// `oci.base` to an approved digest-pinned platform image transactionally.
#[must_use]
pub fn parse_repository_oci_images(source: &[u8]) -> ParsedRepositoryOciImages {
    let source_hash = hash(source);
    let text = match std::str::from_utf8(source) {
        Ok(text) => text,
        Err(error) => {
            return ParsedRepositoryOciImages {
                hash: source_hash,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_utf8"),
                    path: None,
                    message: error.to_string(),
                }],
            };
        }
    };
    let config = match toml::from_str::<RepositoryOciImagesConfig>(text) {
        Ok(config) => config,
        Err(error) => {
            return ParsedRepositoryOciImages {
                hash: source_hash,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_toml"),
                    path: error
                        .span()
                        .map(|span| format!("bytes {}..{}", span.start, span.end)),
                    message: error.message().to_owned(),
                }],
            };
        }
    };
    let diagnostics = validate_repository_oci_images(&config);
    ParsedRepositoryOciImages {
        hash: source_hash,
        config: diagnostics.is_empty().then_some(config),
        diagnostics,
    }
}

/// Parses and validates a repository's root-level `heph.gateways.toml`.
///
/// This only accepts declarative, non-secret gateway intent. Installing the
/// immutable gateway revision and selecting its exact capability bindings are
/// control-plane actions; opening a listener is deliberately outside this
/// parser.
#[must_use]
pub fn parse_repository_gateways(source: &[u8]) -> ParsedRepositoryGateways {
    let source_hash = hash(source);
    let text = match std::str::from_utf8(source) {
        Ok(text) => text,
        Err(error) => {
            return ParsedRepositoryGateways {
                hash: source_hash,
                normalized_hash: None,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_utf8"),
                    path: None,
                    message: error.to_string(),
                }],
            };
        }
    };
    let config = match toml::from_str::<RepositoryGatewaysConfig>(text) {
        Ok(config) => config,
        Err(error) => {
            return ParsedRepositoryGateways {
                hash: source_hash,
                normalized_hash: None,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_toml"),
                    path: error
                        .span()
                        .map(|span| format!("bytes {}..{}", span.start, span.end)),
                    message: error.message().to_owned(),
                }],
            };
        }
    };
    let mut diagnostics = validate_repository_gateways(&config);
    let normalized_hash = if diagnostics.is_empty() {
        let normalized = canonical_repository_gateways(&config);
        toml::to_string(&normalized).map_or_else(
            |_| {
                diagnostics.push(Diagnostic {
                    code: String::from("normalization_failed"),
                    path: None,
                    message: String::from("validated gateway manifest could not be normalized"),
                });
                None
            },
            |normalized| Some(hash(normalized.as_bytes())),
        )
    } else {
        None
    };
    ParsedRepositoryGateways {
        hash: source_hash,
        normalized_hash,
        config: diagnostics.is_empty().then_some(config),
        diagnostics,
    }
}

/// Returns the canonical declaration used for repository gateway identity.
///
/// The parser deliberately returns the original validated declaration so
/// source-facing callers retain their input shape. Persistence code that
/// needs an immutable normalized snapshot should use this canonical clone.
/// Serialize this value as canonical TOML when reproducing
/// [`ParsedRepositoryGateways::normalized_hash`]; that hash intentionally
/// retains the parser's existing TOML serialization contract.
#[must_use]
pub fn canonical_repository_gateways(
    config: &RepositoryGatewaysConfig,
) -> RepositoryGatewaysConfig {
    normalized_repository_gateways(config.clone())
}

pub fn hash(source: &[u8]) -> ConfigHash {
    let digest = Sha256::digest(source);
    let mut value = String::with_capacity(64);
    for byte in digest {
        write!(&mut value, "{byte:02x}").expect("writing to a String cannot fail");
    }
    ConfigHash::from_value(value)
}

fn normalized_config(mut config: AgentConfig) -> AgentConfig {
    for declaration in &mut config.capability_slots {
        declaration.required_operations.sort_unstable();
        declaration.optional_operations.sort_unstable();
    }
    config
        .capability_slots
        .sort_unstable_by(|left, right| left.key.cmp(&right.key));
    config
}

fn normalized_repository_gateways(
    mut config: RepositoryGatewaysConfig,
) -> RepositoryGatewaysConfig {
    for gateway in &mut config.gateways {
        gateway.secret_slots.sort_unstable();
        gateway
            .mailbox_publication_slots
            .sort_unstable_by(|left, right| left.key.cmp(&right.key));
        for route in &mut gateway.routes {
            route.methods.sort_unstable();
        }
        gateway
            .routes
            .sort_unstable_by(|left, right| left.path.cmp(&right.path));
    }
    config
        .gateways
        .sort_unstable_by(|left, right| left.name.cmp(&right.name));
    config
}
