use super::{helpers::git, ui_manifest};
use agent_config::ParsedConfig;
use forge_domain::{CommitSha, GitRef, RefUpdate};
use forge_service::ForgeRepositoryError;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug)]
pub struct InspectedUpdate {
    pub git_ref: GitRef,
    pub commit: CommitSha,
    pub parsed: Option<ParsedConfig>,
    pub ui_manifest: Option<ui_manifest::UiManifestInspection>,
    pub repository_oci_images: Option<Vec<InspectedRepositoryOciImage>>,
}

#[derive(Debug)]
pub struct InspectedRepositoryOciImage {
    pub key: String,
    pub display_name: String,
    pub dockerfile_path: String,
    pub context_path: String,
    pub context_digest: String,
    pub base_key: String,
}

pub fn inspect_updates(
    repository_path: &Path,
    updates: &[RefUpdate],
) -> Result<Vec<InspectedUpdate>, ForgeRepositoryError> {
    let repository = gix::open(repository_path).map_err(git)?;
    updates
        .iter()
        .filter_map(|update| {
            update.new_commit.as_ref().map(|commit| {
                let object_id = gix::ObjectId::from_hex(commit.as_str().as_bytes()).map_err(git)?;
                let object = repository.find_object(object_id).map_err(git)?;
                let tree = object.try_into_commit().map_err(git)?.tree().map_err(git)?;
                let parsed = tree
                    .lookup_entry_by_path("agent.toml")
                    .map_err(git)?
                    .map(|entry| entry.object().map_err(git))
                    .transpose()?
                    .map(|object| {
                        object
                            .try_into_blob()
                            .map(|blob| agent_config::parse(&blob.data))
                            .map_err(git)
                    })
                    .transpose()?;
                let ui_manifest = ui_manifest::inspect_repository_ui(&repository, &tree)?;
                let repository_oci_images = inspect_repository_oci_images(&tree)?;
                Ok(InspectedUpdate {
                    git_ref: update.git_ref.clone(),
                    commit: commit.clone(),
                    parsed,
                    ui_manifest,
                    repository_oci_images,
                })
            })
        })
        .collect()
}

pub fn inspect_repository_oci_images(
    tree: &gix::Tree<'_>,
) -> Result<Option<Vec<InspectedRepositoryOciImage>>, ForgeRepositoryError> {
    let Some(entry) = tree.lookup_entry_by_path("heph.images.toml").map_err(git)? else {
        return Ok(None);
    };
    if !entry.mode().is_blob() {
        return Err(ForgeRepositoryError::InvalidMetadata(
            "heph.images.toml must be a regular file",
        ));
    }
    let manifest = entry.object().map_err(git)?.try_into_blob().map_err(git)?;
    let parsed = agent_config::parse_repository_oci_images(&manifest.data);
    let Some(config) = parsed.config else {
        return Ok(None);
    };
    config
        .images
        .into_iter()
        .map(|image| inspect_repository_oci_image(tree, image))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

pub fn inspect_repository_oci_image(
    tree: &gix::Tree<'_>,
    image: agent_config::RepositoryOciImageConfig,
) -> Result<InspectedRepositoryOciImage, ForgeRepositoryError> {
    let dockerfile = tree
        .lookup_entry_by_path(&image.build.dockerfile)
        .map_err(git)?
        .ok_or(ForgeRepositoryError::InvalidMetadata(
            "repository OCI image Dockerfile does not exist in its source commit",
        ))?;
    if !dockerfile.mode().is_blob() {
        return Err(ForgeRepositoryError::InvalidMetadata(
            "repository OCI image Dockerfile must be a regular file, not a symlink",
        ));
    }
    let context = if image.build.context == "." {
        tree.clone()
    } else {
        let entry = tree
            .lookup_entry_by_path(&image.build.context)
            .map_err(git)?
            .ok_or(ForgeRepositoryError::InvalidMetadata(
                "repository OCI image context does not exist in its source commit",
            ))?;
        if !entry.mode().is_tree() {
            return Err(ForgeRepositoryError::InvalidMetadata(
                "repository OCI image context must be a directory, not a symlink",
            ));
        }
        entry.object().map_err(git)?.try_into_tree().map_err(git)?
    };
    validate_repository_oci_image_context(&context)?;
    let context_digest = format!("sha256:{:x}", Sha256::digest(&context.data));
    Ok(InspectedRepositoryOciImage {
        key: image.key,
        display_name: image.display_name,
        dockerfile_path: image.build.dockerfile,
        context_path: image.build.context,
        context_digest,
        base_key: image
            .build
            .base
            .key
            .ok_or(ForgeRepositoryError::InvalidMetadata(
                "repository OCI image base must select an execution catalog image",
            ))?,
    })
}

pub fn validate_repository_oci_image_context(
    tree: &gix::Tree<'_>,
) -> Result<(), ForgeRepositoryError> {
    for entry in tree.iter() {
        let entry = entry.map_err(git)?;
        if entry.mode().is_link() || entry.mode().is_commit() {
            return Err(ForgeRepositoryError::InvalidMetadata(
                "repository OCI image contexts may not contain symlinks or submodules",
            ));
        }
        if entry.mode().is_tree() {
            let child = entry.object().map_err(git)?.try_into_tree().map_err(git)?;
            validate_repository_oci_image_context(&child)?;
        }
    }
    Ok(())
}
