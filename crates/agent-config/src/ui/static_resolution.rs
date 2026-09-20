//! Pure resolution of static UI declarations to immutable release artifacts.
//!
//! This module deliberately has no database or storage access.  Publication
//! supplies the already imported artifact identities and persists the result.

use super::{RepositoryUisConfig, UiContent};
use release_domain::ui::{UiKey, UiMediaType, UiRoutePath};
use release_domain::{ArtifactKind, ArtifactPath, ReleaseArtifactId};
use std::{collections::BTreeMap, fmt};

/// One trusted artifact candidate supplied by release publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticArtifactCandidate {
    /// Normalized release-relative artifact path.
    pub path: ArtifactPath,
    /// Immutable release artifact identity.
    pub id: ReleaseArtifactId,
    /// Imported artifact kind.
    pub kind: ArtifactKind,
    /// MIME type recorded by the build output declaration.
    pub media_type: String,
}

/// One resolved static file binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStaticFile {
    /// UI-relative route.
    pub route: UiRoutePath,
    /// Exact immutable release artifact identity.
    pub artifact_id: ReleaseArtifactId,
    /// Validated UI MIME type.
    pub media_type: UiMediaType,
}

/// One resolved static UI declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStaticUi {
    /// Stable UI key.
    pub key: UiKey,
    /// Route-to-artifact bindings in manifest order.
    pub files: Vec<ResolvedStaticFile>,
}

/// All static UIs resolved from one validated repository manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStaticUis {
    /// Static declarations in manifest order. Managed-service UIs are omitted.
    pub uis: Vec<ResolvedStaticUi>,
}

/// Redacted failure from static UI resolution.
///
/// Variants contain only bounded manifest/candidate indexes. They never echo
/// repository paths or MIME values from untrusted source or build metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticResolutionError {
    /// The complete UI manifest failed the existing aggregate validator.
    InvalidManifest {
        /// Number of validation diagnostics.
        diagnostic_count: usize,
    },
    /// Two candidates expose the same path.
    DuplicateCandidatePath {
        /// Index of the later candidate.
        candidate_index: usize,
        /// Index of the first candidate with this path.
        first_index: usize,
    },
    /// Two candidates expose the same immutable ID.
    DuplicateCandidateId {
        /// Index of the later candidate.
        candidate_index: usize,
        /// Index of the first candidate with this ID.
        first_index: usize,
    },
    /// A declared static file has no candidate.
    MissingArtifact {
        /// Index of the UI declaration.
        ui_index: usize,
        /// Index of the file declaration within the UI.
        file_index: usize,
    },
    /// A declared static file resolved to a non-file artifact kind.
    WrongArtifactKind {
        /// Index of the UI declaration.
        ui_index: usize,
        /// Index of the file declaration within the UI.
        file_index: usize,
        /// Index of the supplied candidate.
        candidate_index: usize,
    },
    /// The build-recorded MIME differs from the UI declaration.
    MediaTypeMismatch {
        /// Index of the UI declaration.
        ui_index: usize,
        /// Index of the file declaration within the UI.
        file_index: usize,
        /// Index of the supplied candidate.
        candidate_index: usize,
    },
}

impl fmt::Display for StaticResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest { diagnostic_count } => write!(
                formatter,
                "static UI manifest validation failed ({diagnostic_count} diagnostics)"
            ),
            Self::DuplicateCandidatePath {
                candidate_index,
                first_index,
            } => write!(
                formatter,
                "duplicate static artifact candidate path at candidates[{candidate_index}] (first at candidates[{first_index}])"
            ),
            Self::DuplicateCandidateId {
                candidate_index,
                first_index,
            } => write!(
                formatter,
                "duplicate static artifact candidate ID at candidates[{candidate_index}] (first at candidates[{first_index}])"
            ),
            Self::MissingArtifact {
                ui_index,
                file_index,
            } => write!(
                formatter,
                "static artifact is unavailable at uis[{ui_index}].content.files[{file_index}]"
            ),
            Self::WrongArtifactKind {
                ui_index,
                file_index,
                candidate_index,
            } => write!(
                formatter,
                "static artifact has an unsupported kind at uis[{ui_index}].content.files[{file_index}] (candidate {candidate_index})"
            ),
            Self::MediaTypeMismatch {
                ui_index,
                file_index,
                candidate_index,
            } => write!(
                formatter,
                "static artifact MIME does not match at uis[{ui_index}].content.files[{file_index}] (candidate {candidate_index})"
            ),
        }
    }
}

impl std::error::Error for StaticResolutionError {}

/// Resolves static UI declarations to caller-supplied immutable artifact IDs.
///
/// Managed-service declarations are intentionally omitted and resolved by a
/// separate release-agent binding operation. The caller must supply candidates
/// from the exact release being assembled; this pure function does not infer
/// release ownership from an ID.
///
/// # Errors
///
/// Returns a redacted, index-only error if aggregate UI validation fails, the
/// candidate set is ambiguous, or a static declaration cannot be resolved.
pub fn resolve_static_uis(
    config: &RepositoryUisConfig,
    candidates: &[StaticArtifactCandidate],
) -> Result<ResolvedStaticUis, StaticResolutionError> {
    let diagnostics = super::validation::validate(config);
    if !diagnostics.is_empty() {
        return Err(StaticResolutionError::InvalidManifest {
            diagnostic_count: diagnostics.len(),
        });
    }

    let mut paths: BTreeMap<ArtifactPath, usize> = BTreeMap::new();
    let mut ids = BTreeMap::new();
    for (candidate_index, candidate) in candidates.iter().enumerate() {
        if let Some(first_index) = paths.insert(candidate.path.clone(), candidate_index) {
            return Err(StaticResolutionError::DuplicateCandidatePath {
                candidate_index,
                first_index,
            });
        }
        if let Some(first_index) = ids.insert(candidate.id, candidate_index) {
            return Err(StaticResolutionError::DuplicateCandidateId {
                candidate_index,
                first_index,
            });
        }
    }

    let mut resolved = Vec::new();
    for (ui_index, ui) in config.uis.iter().enumerate() {
        let UiContent::Static { files, .. } = &ui.content else {
            continue;
        };
        let mut resolved_files = Vec::with_capacity(files.len());
        for (file_index, file) in files.iter().enumerate() {
            let Some(&candidate_index) = paths.get(&file.artifact) else {
                return Err(StaticResolutionError::MissingArtifact {
                    ui_index,
                    file_index,
                });
            };
            let candidate = &candidates[candidate_index];
            if candidate.kind != ArtifactKind::File {
                return Err(StaticResolutionError::WrongArtifactKind {
                    ui_index,
                    file_index,
                    candidate_index,
                });
            }
            if candidate.media_type != file.media_type.as_str() {
                return Err(StaticResolutionError::MediaTypeMismatch {
                    ui_index,
                    file_index,
                    candidate_index,
                });
            }
            resolved_files.push(ResolvedStaticFile {
                route: file.route.clone(),
                artifact_id: candidate.id,
                media_type: file.media_type,
            });
        }
        resolved.push(ResolvedStaticUi {
            key: ui.key.clone(),
            files: resolved_files,
        });
    }
    Ok(ResolvedStaticUis { uis: resolved })
}

#[cfg(test)]
mod tests {
    use super::{
        ResolvedStaticUis, StaticArtifactCandidate, StaticResolutionError, resolve_static_uis,
    };
    use crate::parse_repository_uis;
    use release_domain::{ArtifactKind, ArtifactPath, ReleaseArtifactId};
    use uuid::Uuid;

    fn config(source: &str) -> crate::ui::RepositoryUisConfig {
        parse_repository_uis(source.as_bytes())
            .config
            .expect("test manifest is valid")
    }

    fn id(value: u128) -> ReleaseArtifactId {
        ReleaseArtifactId::from_uuid(Uuid::from_u128(value))
    }

    fn candidate(
        path: &str,
        value: u128,
        kind: ArtifactKind,
        media_type: &str,
    ) -> StaticArtifactCandidate {
        StaticArtifactCandidate {
            path: ArtifactPath::parse(path).expect("test path"),
            id: id(value),
            kind,
            media_type: media_type.to_owned(),
        }
    }

    fn static_config(files: &str) -> crate::ui::RepositoryUisConfig {
        config(&format!(
            "version = 1\n\n[[uis]]\nkey = \"static-ui\"\nscope = \"global\"\nlabel = \"Static UI\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"static-ui\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n{files}"
        ))
    }

    #[test]
    fn retains_exact_ids_for_multiple_files() {
        let config = static_config(
            "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n\n[[uis.content.files]]\nroute = \"app.js\"\nartifact = \"dist/app.js\"\nmedia_type = \"text/javascript\"\n",
        );
        let result = resolve_static_uis(
            &config,
            &[
                candidate("dist/index.html", 1, ArtifactKind::File, "text/html"),
                candidate("dist/app.js", 2, ArtifactKind::File, "text/javascript"),
            ],
        )
        .expect("static files resolve");
        // Parsing normalizes static files by route, so app.js precedes
        // index.html even though the candidate list uses the source order.
        assert_eq!(result.uis[0].files[0].artifact_id, id(2));
        assert_eq!(result.uis[0].files[1].artifact_id, id(1));
    }

    #[test]
    fn missing_artifact_is_redacted_and_indexed() {
        let error = resolve_static_uis(
            &static_config(
                "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
            ),
            &[],
        )
        .expect_err("missing candidate");
        assert_eq!(
            error,
            StaticResolutionError::MissingArtifact {
                ui_index: 0,
                file_index: 0,
            }
        );
        assert!(!error.to_string().contains("dist/index.html"));
        assert!(!error.to_string().contains("text/html"));
    }

    #[test]
    fn non_file_kinds_are_rejected() {
        for kind in [
            ArtifactKind::Executable,
            ArtifactKind::Manifest,
            ArtifactKind::BuildLog,
        ] {
            let error = resolve_static_uis(
                &static_config(
                    "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
                ),
                &[candidate("dist/index.html", 1, kind, "text/html")],
            )
            .expect_err("non-file candidate");
            assert!(matches!(
                error,
                StaticResolutionError::WrongArtifactKind { .. }
            ));
        }
    }

    #[test]
    fn mime_mismatch_including_default_octet_stream_is_rejected() {
        let error = resolve_static_uis(
            &static_config(
                "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
            ),
            &[candidate(
                "dist/index.html",
                1,
                ArtifactKind::File,
                "application/octet-stream",
            )],
        )
        .expect_err("MIME mismatch");
        assert!(matches!(
            error,
            StaticResolutionError::MediaTypeMismatch { .. }
        ));
    }

    #[test]
    fn duplicate_paths_and_ids_are_ambiguous() {
        let config = static_config(
            "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
        );
        let error = resolve_static_uis(
            &config,
            &[
                candidate("dist/index.html", 1, ArtifactKind::File, "text/html"),
                candidate("dist/index.html", 2, ArtifactKind::File, "text/html"),
            ],
        )
        .expect_err("duplicate path");
        assert!(matches!(
            error,
            StaticResolutionError::DuplicateCandidatePath { .. }
        ));

        let error = resolve_static_uis(
            &config,
            &[
                candidate("dist/index.html", 1, ArtifactKind::File, "text/html"),
                candidate("dist/app.js", 1, ArtifactKind::File, "text/javascript"),
            ],
        )
        .expect_err("duplicate ID");
        assert!(matches!(
            error,
            StaticResolutionError::DuplicateCandidateId { .. }
        ));
    }

    #[test]
    fn malformed_typed_manifest_fails_aggregate_validation() {
        let mut config = static_config(
            "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
        );
        config.version = 99;
        let error = resolve_static_uis(&config, &[]).expect_err("invalid version");
        assert!(matches!(
            error,
            StaticResolutionError::InvalidManifest { .. }
        ));
    }

    #[test]
    fn managed_uis_are_omitted() {
        let config = config(
            "version = 1\n\n[[uis]]\nkey = \"managed-ui\"\nscope = \"global\"\nlabel = \"Managed UI\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"managed-ui\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[uis.content]\nkind = \"managed_service\"\ngateway_name = \"service\"\nroute = \"/service\"\nentrypoint = \"index.html\"\n",
        );
        assert_eq!(
            resolve_static_uis(&config, &[]).expect("managed ignored"),
            ResolvedStaticUis { uis: Vec::new() }
        );
    }
}
