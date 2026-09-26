use agent_config::ui::{RepositoryUisConfig, UiContent};
use agent_config::{ConfigHash, ParsedRepositoryGateways};
use forge_service::ForgeRepositoryError;

use super::types::UiManifestStatus;
use super::types::{
    GatewayManifestInspection, UiManifestEntryKind, UiManifestInspection, UiManifestLocation,
};
use super::validation::{diagnostic, entry_kind, invalid_inspection, redact_diagnostics};
use super::{
    GATEWAY_MANIFEST_PATH, MAX_GATEWAY_SOURCE_BYTES, MAX_UI_SOURCE_BYTES, UI_MANIFEST_PATH,
};
use crate::repository::git;

/// Inspects one exact commit tree for its bounded UI declaration.
///
/// A missing `heph.ui.toml` is the legacy result and returns `Ok(None)`.
/// Invalid source-tree entries and oversized manifests return a redacted
/// invalid result.  Missing or corrupt Git objects remain fatal inspection
/// errors because they indicate repository/object-store failure.
pub fn inspect_repository_ui(
    repository: &gix::Repository,
    tree: &gix::Tree<'_>,
) -> Result<Option<UiManifestInspection>, ForgeRepositoryError> {
    let Some(entry) = tree.lookup_entry_by_path(UI_MANIFEST_PATH).map_err(git)? else {
        return Ok(None);
    };

    let entry_kind = entry_kind(entry.mode())
        .ok_or_else(|| git("repository UI manifest has an unknown Git entry mode"))?;
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

    let header = repository.find_header(object_id).map_err(git)?;
    if header.kind() != gix::objs::Kind::Blob {
        return Err(git("repository UI manifest object kind is corrupt"));
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

    let manifest = entry.object().map_err(git)?.try_into_blob().map_err(git)?;
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

fn inspect_required_gateways(
    repository: &gix::Repository,
    tree: &gix::Tree<'_>,
) -> Result<GatewayManifestInspection, ForgeRepositoryError> {
    let Some(entry) = tree
        .lookup_entry_by_path(GATEWAY_MANIFEST_PATH)
        .map_err(git)?
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
        .ok_or_else(|| git("repository gateway manifest has an unknown Git entry mode"))?;
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
    let header = repository.find_header(object_id).map_err(git)?;
    if header.kind() != gix::objs::Kind::Blob {
        return Err(git("repository gateway manifest object kind is corrupt"));
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
    let manifest = entry.object().map_err(git)?.try_into_blob().map_err(git)?;
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

pub(super) fn requires_gateway_manifest(config: &RepositoryUisConfig) -> bool {
    config
        .uis
        .iter()
        .any(|ui| !ui.apis.is_empty() || matches!(ui.content, UiContent::ManagedService { .. }))
}
