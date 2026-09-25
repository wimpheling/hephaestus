//! Repository image and gateway manifest validation.
use super::validation_shared::{diagnostic, valid_key, valid_relative_path};
use crate::{
    Diagnostic, ImageSelection, REPOSITORY_GATEWAYS_VERSION, REPOSITORY_OCI_IMAGES_VERSION,
    RepositoryGatewaysConfig, RepositoryOciImagesConfig,
};
use gateway_domain::GatewayName;
use std::collections::HashSet;
pub fn validate_repository_oci_images(config: &RepositoryOciImagesConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if config.version != REPOSITORY_OCI_IMAGES_VERSION {
        diagnostic(
            &mut diagnostics,
            "unsupported_repository_oci_images_version",
            "version",
            format!(
                "repository OCI image version {} is unsupported; expected {REPOSITORY_OCI_IMAGES_VERSION}",
                config.version
            ),
        );
        return diagnostics;
    }
    if config.images.len() > 64 {
        diagnostic(
            &mut diagnostics,
            "too_many_repository_oci_images",
            "images",
            "a repository may define at most 64 OCI images",
        );
    }
    let mut keys = HashSet::new();
    for (index, image) in config.images.iter().enumerate() {
        if !valid_key(&image.key, 64) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_oci_image_key",
                format!("images[{index}].key"),
                "OCI image keys must be lowercase and at most 64 characters",
            );
        } else if !keys.insert(&image.key) {
            diagnostic(
                &mut diagnostics,
                "duplicate_repository_oci_image_key",
                format!("images[{index}].key"),
                "OCI image keys must be unique within a repository",
            );
        }
        if image.display_name.trim().is_empty() || image.display_name.len() > 200 {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_oci_image_display_name",
                format!("images[{index}].display_name"),
                "display names must contain 1 to 200 characters",
            );
        }
        if !valid_repository_oci_image_path(&image.build.dockerfile, false) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_oci_image_dockerfile",
                format!("images[{index}].build.dockerfile"),
                "dockerfile must be a safe repository-relative path",
            );
        }
        if !valid_repository_oci_image_path(&image.build.context, true) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_oci_image_context",
                format!("images[{index}].build.context"),
                "context must be a safe repository-relative path or .",
            );
        }
        if !matches!(&image.build.base.key, Some(key) if valid_key(key, 64))
            || image.build.base.project_image.is_some()
        {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_oci_image_base",
                format!("images[{index}].build.base.key"),
                "base image keys must be lowercase and at most 64 characters",
            );
        }
    }
    diagnostics
}

pub fn validate_repository_gateways(config: &RepositoryGatewaysConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if config.version != REPOSITORY_GATEWAYS_VERSION {
        diagnostic(
            &mut diagnostics,
            "unsupported_repository_gateways_version",
            "version",
            format!(
                "repository gateway version {} is unsupported; expected {REPOSITORY_GATEWAYS_VERSION}",
                config.version
            ),
        );
        return diagnostics;
    }
    if config.gateways.len() > 64 {
        diagnostic(
            &mut diagnostics,
            "too_many_repository_gateways",
            "gateways",
            "a repository may define at most 64 gateways",
        );
    }
    let mut names = HashSet::new();
    for (gateway_index, gateway) in config.gateways.iter().enumerate() {
        if GatewayName::parse(gateway.name.clone()).is_err() {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_gateway_name",
                format!("gateways[{gateway_index}].name"),
                "gateway names must be lowercase and at most 64 characters",
            );
        } else if !names.insert(&gateway.name) {
            diagnostic(
                &mut diagnostics,
                "duplicate_repository_gateway_name",
                format!("gateways[{gateway_index}].name"),
                "gateway names must be unique within a repository",
            );
        }
        if gateway.mailbox_publication_slots.len() > 32 {
            diagnostic(
                &mut diagnostics,
                "too_many_repository_gateway_mailbox_publication_slots",
                format!("gateways[{gateway_index}].mailbox_publication_slots"),
                "a gateway may declare at most 32 mailbox publication slots",
            );
        }
        let mut mailbox_publication_slot_keys = HashSet::new();
        for (slot_index, slot) in gateway.mailbox_publication_slots.iter().enumerate() {
            let path = format!("gateways[{gateway_index}].mailbox_publication_slots[{slot_index}]");
            if slot.to_declaration().is_err() {
                diagnostic(
                    &mut diagnostics,
                    "invalid_repository_gateway_mailbox_publication_slot",
                    path.clone(),
                    "mailbox publication slots require a bounded lowercase key and a 1 to 512 character purpose",
                );
            }
            if !mailbox_publication_slot_keys.insert(&slot.key) {
                diagnostic(
                    &mut diagnostics,
                    "duplicate_repository_gateway_mailbox_publication_slot",
                    format!("{path}.key"),
                    "mailbox publication slot keys must be unique within a gateway",
                );
            }
        }
        match gateway.to_declaration() {
            Ok(declaration) => {
                if declaration.validate().is_err() {
                    diagnostic(
                        &mut diagnostics,
                        "invalid_repository_gateway_declaration",
                        format!("gateways[{gateway_index}]"),
                        "gateway declarations must use a supported HTTP contract with matching service settings, unique bounded routes, and symbolic secret slots",
                    );
                }
            }
            Err(_) => diagnostic(
                &mut diagnostics,
                "invalid_repository_gateway_declaration",
                format!("gateways[{gateway_index}]"),
                "gateway declarations must use valid names, routes, and non-empty method sets",
            ),
        }
    }
    diagnostics
}

fn valid_repository_oci_image_path(value: &str, permit_current_directory: bool) -> bool {
    if permit_current_directory && value == "." {
        return true;
    }
    valid_relative_path(value)
}

pub fn validate_image_selection(
    diagnostics: &mut Vec<Diagnostic>,
    field: &str,
    selection: &ImageSelection,
    permit_project_image: bool,
) {
    match (&selection.key, &selection.project_image) {
        (Some(key), None) if valid_key(key, 64) => {}
        (None, Some(key)) if permit_project_image && valid_key(key, 64) => {}
        (None, Some(_)) => diagnostic(
            diagnostics,
            "project_image_not_permitted",
            format!("{field}.project_image"),
            "project-owned OCI images may be selected only by an isolated build contract",
        ),
        _ => diagnostic(
            diagnostics,
            "invalid_image_selection",
            field,
            "image must select exactly one lowercase catalog key or project_image key",
        ),
    }
}
