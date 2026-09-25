//! Stable build request identity hashing.

use sha2::{Digest, Sha256};

/// Domain separator for build identities that include a repository UI.
const UI_BUILD_DEFINITION_DOMAIN: &[u8] = b"hephaestus.build-definition-with-ui.v1";

/// Hashes the canonical JSON representation used for the existing build
/// definition identity.
///
/// Keeping this serialization here preserves the pre-UI identity exactly;
/// callers that have no valid UI snapshot must continue using this result.
///
/// # Errors
///
/// Returns the serialization error if `build` cannot be represented as JSON.
pub fn base_build_definition_hash(
    build: &super::BuildConfig,
) -> Result<[u8; 32], serde_json::Error> {
    let bytes = serde_json::to_vec(build)?;
    Ok(Sha256::digest(bytes).into())
}

/// Derives a domain-separated build identity for a valid repository UI.
///
/// The input is `domain || base || ui || gateway-marker`, where the marker is
/// `0` when no normalized gateway configuration is required and `1` followed
/// by the fixed 32-byte gateway hash otherwise.
#[must_use]
pub fn ui_build_definition_hash(
    base_hash: [u8; 32],
    ui_hash: [u8; 32],
    gateway_hash: Option<[u8; 32]>,
) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(
        UI_BUILD_DEFINITION_DOMAIN.len() + 32 + 32 + 1 + gateway_hash.map_or(0, |_| 32),
    );
    bytes.extend_from_slice(UI_BUILD_DEFINITION_DOMAIN);
    bytes.extend_from_slice(&base_hash);
    bytes.extend_from_slice(&ui_hash);
    match gateway_hash {
        Some(gateway_hash) => {
            bytes.push(1);
            bytes.extend_from_slice(&gateway_hash);
        }
        None => bytes.push(0),
    }
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::{base_build_definition_hash, ui_build_definition_hash};
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    fn hex(value: [u8; 32]) -> String {
        let mut output = String::with_capacity(64);
        for byte in value {
            write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
        }
        output
    }

    fn build() -> super::super::BuildConfig {
        serde_json::from_value(serde_json::json!({
            "image": { "key": "builder" },
            "command": "/bin/build",
            "arguments": [],
            "working_directory": "/workspace",
            "resources": { "vcpus": 2, "memory_mib": 256 },
            "network": { "profile": "disabled" },
            "artifacts": [{
                "path": "dist/app",
                "kind": "file",
                "media_type": null
            }],
            "triggers": []
        }))
        .expect("valid build definition")
    }

    #[test]
    fn base_hash_preserves_existing_serialization_identity() {
        let build = build();
        let bytes = serde_json::to_vec(&build).expect("serializable build definition");
        let expected: [u8; 32] = Sha256::digest(bytes).into();
        assert_eq!(base_build_definition_hash(&build).expect("hash"), expected);
        assert_eq!(
            hex(expected),
            "358207c6e3fe44f4fb90f859cc51b6b3a30c025ff8c8beba6435b2341a5ee5ca"
        );
    }

    #[test]
    fn absent_and_present_gateway_hashes_have_fixed_known_vectors() {
        let absent = ui_build_definition_hash([1; 32], [2; 32], None);
        let present = ui_build_definition_hash([1; 32], [2; 32], Some([3; 32]));
        assert_eq!(
            hex(absent),
            "8ffe20f1ef402f455dca3782abc813ee89f95968ec5bdcac26e48b800742561c"
        );
        assert_eq!(
            hex(present),
            "70d20d35c96a007dafe4871b85c5aeab422ddb361feeb49ee4c2cffdd37cd5ed"
        );
        assert_ne!(absent, present);
    }

    #[test]
    fn ui_and_gateway_inputs_are_identity_separators() {
        let base = [1; 32];
        let ui = [2; 32];
        let gateway = [3; 32];
        assert_ne!(
            ui_build_definition_hash(base, ui, Some(gateway)),
            ui_build_definition_hash(base, [4; 32], Some(gateway))
        );
        assert_ne!(
            ui_build_definition_hash(base, ui, Some(gateway)),
            ui_build_definition_hash(base, ui, Some([5; 32]))
        );
        assert_ne!(
            ui_build_definition_hash(base, ui, None),
            ui_build_definition_hash([6; 32], ui, None)
        );
    }
}
