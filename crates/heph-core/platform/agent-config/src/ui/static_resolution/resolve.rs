//! Pure static UI declaration to artifact resolution.

use super::super::{RepositoryUisConfig, UiContent};
use super::model::{
    MAX_STATIC_UI_FILE_BYTES, MAX_STATIC_UI_TOTAL_BYTES, ResolvedStaticFile, ResolvedStaticUi,
    ResolvedStaticUis, StaticArtifactCandidate, StaticResolutionError,
};
use release_domain::{ArtifactKind, ArtifactPath};
use std::collections::{BTreeMap, BTreeSet};

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
    let diagnostics = super::super::validation::validate(config);
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
    let mut referenced_ids = BTreeSet::new();
    let mut referenced_bytes = 0_u64;
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
            if candidate.size_bytes > MAX_STATIC_UI_FILE_BYTES {
                return Err(StaticResolutionError::StaticArtifactTooLarge {
                    ui_index,
                    file_index,
                    candidate_index,
                });
            }
            if referenced_ids.insert(candidate.id) {
                referenced_bytes = referenced_bytes.checked_add(candidate.size_bytes).ok_or(
                    StaticResolutionError::StaticArtifactTotalTooLarge {
                        ui_index,
                        file_index,
                        candidate_index,
                    },
                )?;
                if referenced_bytes > MAX_STATIC_UI_TOTAL_BYTES {
                    return Err(StaticResolutionError::StaticArtifactTotalTooLarge {
                        ui_index,
                        file_index,
                        candidate_index,
                    });
                }
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
