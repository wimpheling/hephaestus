use release_domain::ContentHash;
use serde_json::{Value, json};

use super::fixture::{self, SOURCE};
use crate::{DeploymentExecutionPreparation, MAX_EXECUTION_PREPARATION_BYTES};

fn restore(bytes: &[u8]) -> Result<DeploymentExecutionPreparation, crate::DeploymentError> {
    DeploymentExecutionPreparation::from_canonical_bytes(
        &fixture::intent(SOURCE, 1),
        fixture::command(),
        bytes,
        ContentHash::digest(bytes),
    )
}

#[test]
fn original_canonical_preparation_round_trips() {
    let prepared = fixture::prepared();
    let bytes = prepared.canonical_bytes().unwrap();
    assert_eq!(restore(&bytes).unwrap(), prepared);
    assert_eq!(ContentHash::digest(&bytes), prepared.input_hash().unwrap());
}

#[test]
fn every_import_field_is_compared_after_reconstructing_stable_mapping() {
    let prepared = fixture::prepared();
    let canonical = String::from_utf8(prepared.canonical_bytes().unwrap()).unwrap();
    let original: Value = serde_json::from_str(&canonical).unwrap();
    let changes = [
        ("resource", json!("different")),
        ("operation_id", json!(uuid::Uuid::from_u128(40))),
        (
            "command_key",
            json!(release_domain::ReleaseCommandKey::derive("forged", &[])),
        ),
        ("instance_id", json!(uuid::Uuid::from_u128(41))),
        ("revision_id", json!(uuid::Uuid::from_u128(42))),
        ("project_id", json!(uuid::Uuid::from_u128(43))),
        ("name", json!("changed")),
        ("parameters", json!({"message":"changed"})),
    ];
    for (field, replacement) in changes {
        let bytes = replace_import_field(
            &canonical,
            field,
            &original["instances"][0][field],
            &replacement,
        );
        assert!(restore(&bytes).is_err(), "field {field}");
    }
    for (field, replacement) in [
        ("slot", json!("other")),
        ("guest_path", json!("/other")),
        ("access_mode", json!("read_only")),
        ("volume_id", json!(uuid::Uuid::from_u128(44))),
        ("grant_id", json!(uuid::Uuid::from_u128(45))),
    ] {
        let bytes = replace_import_field(
            &canonical,
            field,
            &original["instances"][0]["volumes"][0][field],
            &replacement,
        );
        assert!(restore(&bytes).is_err(), "volume {field}");
    }
}

fn replace_import_field(
    canonical: &str,
    field: &str,
    original: &Value,
    replacement: &Value,
) -> Vec<u8> {
    // Preserve original object order so a field comparison, rather than an
    // unrelated noncanonical encoding, must reject the forged mapping.
    let split = canonical.find("\"instances\":[").unwrap();
    let old = format!("\"{field}\":{original}");
    assert!(canonical[split..].contains(&old));
    format!(
        "{}{}",
        &canonical[..split],
        canonical[split..].replacen(&old, &format!("\"{field}\":{replacement}"), 1)
    )
    .into_bytes()
}

#[test]
fn malformed_unknown_noncanonical_and_incomplete_storage_is_rejected() {
    let prepared = fixture::prepared();
    let bytes = prepared.canonical_bytes().unwrap();
    for invalid in [
        vec![],
        b"{".to_vec(),
        vec![b' '; MAX_EXECUTION_PREPARATION_BYTES + 1],
    ] {
        assert!(restore(&invalid).is_err());
    }
    assert!(
        DeploymentExecutionPreparation::from_canonical_bytes(
            &fixture::intent(SOURCE, 1),
            fixture::command(),
            &bytes,
            ContentHash::digest(b"wrong fingerprint")
        )
        .is_err()
    );
    let original: Value = serde_json::from_slice(&bytes).unwrap();
    let mut unknown = original;
    unknown["new_authority"] = json!(true);
    assert!(restore(&serde_json::to_vec(&unknown).unwrap()).is_err());
    let mut incomplete = prepared.clone();
    incomplete.instances.pop();
    assert!(restore(&incomplete.canonical_bytes().unwrap()).is_err());
    let mut duplicate = prepared.clone();
    duplicate.instances.push(duplicate.instances[0].clone());
    assert!(restore(&duplicate.canonical_bytes().unwrap()).is_err());
    let mut reordered = prepared.clone();
    reordered.instances.reverse();
    assert!(restore(&reordered.canonical_bytes().unwrap()).is_err());
    assert!(restore(&serde_json::to_vec_pretty(&prepared).unwrap()).is_err());
    let mut future_version = prepared;
    future_version.version = 2;
    assert!(restore(&future_version.canonical_bytes().unwrap()).is_err());
    let unknown_profile = String::from_utf8(bytes)
        .unwrap()
        .replace("\"runtime_named_v1\"", "\"unknown_profile\"");
    assert!(restore(unknown_profile.as_bytes()).is_err());
}

#[test]
fn original_command_and_source_platform_cannot_be_borrowed_or_changed() {
    let prepared = fixture::prepared();
    let bytes = prepared.canonical_bytes().unwrap();
    let other = crate::CommandIdentity::from_identity(
        &fixture::identity(10, 8),
        crate::DeploymentOperation::Install,
    )
    .unwrap();
    assert!(
        DeploymentExecutionPreparation::from_canonical_bytes(
            &fixture::intent(SOURCE, 1),
            other,
            &bytes,
            prepared.input_hash().unwrap()
        )
        .is_err()
    );
    let mut changed: Value = serde_json::from_slice(&bytes).unwrap();
    changed["instances"][0]["source"]["platform"]["version"] = json!("platform/v2");
    assert!(restore(&serde_json::to_vec(&changed).unwrap()).is_err());
}
