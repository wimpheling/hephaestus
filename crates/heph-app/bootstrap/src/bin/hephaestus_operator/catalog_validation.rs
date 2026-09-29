use serde_json::{Map, Value};
use std::{collections::HashSet, error::Error, path::Path};
use uuid::Uuid;

use super::catalog::OciImageCatalogManifest;
use super::catalog::OciImageRecord;

pub fn parse_image_catalog_manifest(
    contents: &str,
    path: &Path,
) -> Result<OciImageCatalogManifest, Box<dyn Error>> {
    let document = serde_json::from_str::<Value>(contents)
        .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))?;
    let object = document
        .as_object()
        .ok_or_else(|| manifest_error("image catalog manifest must be a JSON object"))?;
    let schema_version = required_u64(object, "schema_version")?;
    if schema_version != 1 {
        return Err(manifest_error(format!(
            "unsupported image catalog manifest schema_version {schema_version}; expected 1"
        )));
    }
    let images = required_array(object, "images")?;
    if images.is_empty() {
        return Err(manifest_error(
            "image catalog manifest must contain at least one image",
        ));
    }

    let mut records = Vec::with_capacity(images.len());
    let mut ids = HashSet::with_capacity(images.len());
    let mut keys = HashSet::with_capacity(images.len());
    let mut references = HashSet::with_capacity(images.len());
    for (index, image) in images.iter().enumerate() {
        let record = parse_image_catalog_record(image, index)?;
        if !ids.insert(record.id) {
            return Err(manifest_error(format!(
                "images[{index}] duplicates stable id {}",
                record.id
            )));
        }
        if !keys.insert(record.key.clone()) {
            return Err(manifest_error(format!(
                "images[{index}] duplicates key {}",
                record.key
            )));
        }
        if !references.insert(record.image_reference.clone()) {
            return Err(manifest_error(format!(
                "images[{index}] duplicates image reference {}",
                record.image_reference
            )));
        }
        records.push(record);
    }
    Ok(OciImageCatalogManifest {
        schema_version,
        images: records,
    })
}

// Keep all field validation in one reviewable record parser so malformed
// catalog entries cannot bypass a validation branch by taking another path.
#[allow(clippy::too_many_lines)]
fn parse_image_catalog_record(
    value: &Value,
    index: usize,
) -> Result<OciImageRecord, Box<dyn Error>> {
    let object = value
        .as_object()
        .ok_or_else(|| manifest_error(format!("images[{index}] must be a JSON object")))?;
    let id = Uuid::parse_str(required_string(object, "id")?.as_str())
        .map_err(|error| manifest_error(format!("images[{index}].id is invalid: {error}")))?;
    let key = required_string(object, "key")?;
    validate_key(&key, index)?;
    let display_name = required_string(object, "display_name")?;
    if display_name.trim().is_empty() || display_name.len() > 200 {
        return Err(manifest_error(format!(
            "images[{index}].display_name must contain 1..=200 non-whitespace bytes"
        )));
    }
    let image_reference = required_string(object, "image_reference")?;
    validate_image_reference(&image_reference, index)?;
    let toolchains = required_value(object, "toolchains")?.clone();
    validate_toolchains(&toolchains, index)?;
    let architectures = string_array(object, "architectures", index)?;
    if architectures.is_empty()
        || architectures.iter().any(|architecture| {
            architecture.is_empty()
                || architecture.len() > 32
                || architecture
                    .bytes()
                    .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        })
    {
        return Err(manifest_error(format!(
            "images[{index}].architectures must contain non-empty, bounded values"
        )));
    }
    let availability_state = required_string(object, "availability_state")?;
    if !matches!(
        availability_state.as_str(),
        "available" | "unavailable" | "retired"
    ) {
        return Err(manifest_error(format!(
            "images[{index}].availability_state is invalid"
        )));
    }
    let role = optional_string(object, "role")?.unwrap_or_else(|| String::from("execution"));
    if !matches!(role.as_str(), "execution" | "platform_operation") {
        return Err(manifest_error(format!("images[{index}].role is invalid")));
    }
    let provenance = required_value(object, "provenance")?.clone();
    let provenance_object = provenance.as_object().ok_or_else(|| {
        manifest_error(format!("images[{index}].provenance must be a JSON object"))
    })?;
    let source = required_string(provenance_object, "source")?;
    if source.trim().is_empty() {
        return Err(manifest_error(format!(
            "images[{index}].provenance.source must not be empty"
        )));
    }
    let signature_reference = optional_string(provenance_object, "signature")?;
    let sbom_reference = optional_string(provenance_object, "sbom")?;
    let platform_policy_version = required_string(object, "platform_policy_version")?;
    if platform_policy_version.trim().is_empty() || platform_policy_version.len() > 128 {
        return Err(manifest_error(format!(
            "images[{index}].platform_policy_version must contain 1..=128 bytes"
        )));
    }

    Ok(OciImageRecord {
        id,
        key,
        display_name,
        image_reference,
        toolchains,
        architectures,
        availability_state,
        role,
        provenance,
        signature_reference,
        sbom_reference,
        platform_policy_version,
    })
}

fn validate_key(value: &str, index: usize) -> Result<(), Box<dyn Error>> {
    let valid = (1..=64).contains(&value.len())
        && value.bytes().enumerate().all(|(position, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || ((byte == b'_' || byte == b'-') && position > 0)
        });
    valid.then_some(()).ok_or_else(|| {
        manifest_error(format!(
            "images[{index}].key must be a lowercase catalog identifier"
        ))
    })
}

fn validate_image_reference(value: &str, index: usize) -> Result<(), Box<dyn Error>> {
    let Some((repository, digest)) = value.rsplit_once("@sha256:") else {
        return Err(manifest_error(format!(
            "images[{index}].image_reference must be digest-pinned"
        )));
    };
    let valid_repository = !repository.is_empty()
        && repository == repository.trim()
        && !repository
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace());
    let valid_digest = digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if valid_repository && valid_digest {
        Ok(())
    } else {
        Err(manifest_error(format!(
            "images[{index}].image_reference must end in a lowercase 64-character sha256 digest"
        )))
    }
}

fn validate_toolchains(value: &Value, index: usize) -> Result<(), Box<dyn Error>> {
    let toolchains = value
        .as_array()
        .ok_or_else(|| manifest_error(format!("images[{index}].toolchains must be an array")))?;
    if toolchains.is_empty() {
        return Err(manifest_error(format!(
            "images[{index}].toolchains must contain at least one toolchain"
        )));
    }
    for (toolchain_index, toolchain) in toolchains.iter().enumerate() {
        let object = toolchain.as_object().ok_or_else(|| {
            manifest_error(format!(
                "images[{index}].toolchains[{toolchain_index}] must be an object"
            ))
        })?;
        let name = required_string(object, "name")?;
        let version = required_string(object, "version")?;
        if name.trim().is_empty()
            || name.len() > 64
            || version.trim().is_empty()
            || version.len() > 128
        {
            return Err(manifest_error(format!(
                "images[{index}].toolchains[{toolchain_index}] has invalid name or version"
            )));
        }
    }
    Ok(())
}

fn required_value<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Value, Box<dyn Error>> {
    object
        .get(key)
        .ok_or_else(|| manifest_error(format!("missing required field {key}")))
}

fn required_string(object: &Map<String, Value>, key: &str) -> Result<String, Box<dyn Error>> {
    required_value(object, key)?
        .as_str()
        .map(String::from)
        .ok_or_else(|| manifest_error(format!("field {key} must be a string")))
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(String::from(value)))
            .ok_or_else(|| manifest_error(format!("field {key} must be a string or null"))),
    }
}

fn required_u64(object: &Map<String, Value>, key: &str) -> Result<u64, Box<dyn Error>> {
    required_value(object, key)?
        .as_u64()
        .ok_or_else(|| manifest_error(format!("field {key} must be a positive integer")))
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Vec<Value>, Box<dyn Error>> {
    required_value(object, key)?
        .as_array()
        .ok_or_else(|| manifest_error(format!("field {key} must be an array")))
}

fn string_array(
    object: &Map<String, Value>,
    key: &str,
    index: usize,
) -> Result<Vec<String>, Box<dyn Error>> {
    required_array(object, key)?
        .iter()
        .enumerate()
        .map(|(array_index, value)| {
            value.as_str().map(String::from).ok_or_else(|| {
                manifest_error(format!(
                    "images[{index}].{key}[{array_index}] must be a string"
                ))
            })
        })
        .collect()
}

fn manifest_error(message: impl Into<String>) -> Box<dyn Error> {
    message.into().into()
}
