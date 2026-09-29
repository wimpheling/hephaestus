use super::ArtifactError;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const CURSOR_DOMAIN: &[u8] = b"hephaestus.artifact-cursor.v1\0";

pub(super) fn encode_cursor(
    key: &[u8; 32],
    actor_id: Uuid,
    artifact_id: Uuid,
    offset: u64,
) -> String {
    let digest = cursor_digest(key, actor_id, artifact_id, offset);
    format!("v1.{offset}.{}", encode_hex(&digest))
}

pub(super) fn decode_cursor(
    value: &str,
    key: &[u8; 32],
    actor_id: &Uuid,
    artifact_id: Uuid,
) -> Result<u64, ArtifactError> {
    let mut parts = value.split('.');
    if parts.next() != Some("v1") {
        return Err(ArtifactError::InvalidArgument);
    }
    let offset = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or(ArtifactError::InvalidArgument)?;
    let supplied = parts.next().ok_or(ArtifactError::InvalidArgument)?;
    if parts.next().is_some()
        || supplied != encode_hex(&cursor_digest(key, *actor_id, artifact_id, offset))
    {
        return Err(ArtifactError::InvalidArgument);
    }
    Ok(offset)
}

fn cursor_digest(key: &[u8; 32], actor_id: Uuid, artifact_id: Uuid, offset: u64) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(CURSOR_DOMAIN);
    digest.update(key);
    digest.update(actor_id.as_bytes());
    digest.update(artifact_id.as_bytes());
    digest.update(offset.to_be_bytes());
    digest.finalize().into()
}

fn encode_hex(value: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}
