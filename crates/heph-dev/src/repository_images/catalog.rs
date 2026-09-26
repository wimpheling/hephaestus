use crate::process::{DevError, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub images: Vec<CatalogImage>,
}

#[derive(Debug, Deserialize)]
pub struct CatalogImage {
    pub key: String,
    pub image_reference: String,
    #[serde(default = "execution_role")]
    pub role: String,
}

// The catalog importer has always treated an omitted role as `execution`.
// Keep that bounded compatibility for immutable catalogs installed before the
// generator began writing the role explicitly.
pub fn execution_role() -> String {
    String::from("execution")
}

pub fn operational_image(catalog: &Catalog, key: &str) -> Result<String> {
    let image = catalog
        .images
        .iter()
        .find(|image| image.key == key && image.role == "platform_operation")
        .ok_or_else(|| {
            DevError::Invalid(format!(
                "installed platform catalog lacks required operational image {key}"
            ))
        })?;
    valid_reference(&image.image_reference)
        .then_some(image.image_reference.clone())
        .ok_or_else(|| DevError::Invalid(format!("operational image {key} is not digest-pinned")))
}

pub fn valid_reference(value: &str) -> bool {
    value
        .rsplit_once("@sha256:")
        .is_some_and(|(repository, digest)| {
            !repository.is_empty()
                && digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

pub fn validate_revision(value: &str) -> Result<()> {
    (matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(())
    .ok_or_else(|| {
        DevError::Invalid(String::from(
            "revision must be a lowercase 40- or 64-character commit SHA",
        ))
    })
}

pub fn layout_tag(path: &Path) -> Result<String> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?).map_err(|error| {
        DevError::Invalid(format!(
            "platform release metadata is invalid ({}): {error}",
            path.display()
        ))
    })?;
    let tag = value
        .get("layout_tag")
        .and_then(serde_json::Value::as_str)
        .filter(|tag| tag.starts_with("heph-sha256-") && tag.len() == 76)
        .ok_or_else(|| {
            DevError::Invalid(format!(
                "platform release metadata lacks an immutable layout tag: {}",
                path.display()
            ))
        })?;
    Ok(String::from(tag))
}

pub fn execution_layouts(catalog: &Catalog, release: &Path) -> Result<BTreeMap<String, String>> {
    let mut layouts = BTreeMap::new();
    for image in catalog
        .images
        .iter()
        .filter(|image| image.role == "execution")
    {
        let layout = release.join(&image.key).join("image");
        if !layout.join("index.json").is_file() || !layout.join("oci-layout").is_file() {
            return Err(DevError::Invalid(format!(
                "platform release is missing an execution base layout: {}",
                layout.display()
            )));
        }
        layouts.insert(image.image_reference.clone(), layout.display().to_string());
    }
    (!layouts.is_empty()).then_some(layouts).ok_or_else(|| {
        DevError::Invalid(String::from(
            "installed platform catalog lacks execution base images",
        ))
    })
}
