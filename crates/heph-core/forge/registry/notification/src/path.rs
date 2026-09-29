//! Bounded OCI repository and reference validation.

use crate::NotificationError;
use registry_domain::Sha256Digest;
use std::str::FromStr;

pub fn is_canonical_repository_path(value: &str) -> bool {
    (1..=255).contains(&value.len())
        && value.split('/').all(|component| {
            !component.is_empty()
                && component.len() <= 128
                && component.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
                })
                && !component.ends_with(['.', '_', '-'])
        })
}

pub fn validate_reference(value: String) -> Result<String, NotificationError> {
    let is_digest = Sha256Digest::from_str(&value).is_ok();
    let is_tag = (1..=128).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric()
                || byte == b'_'
                || (index > 0 && matches!(byte, b'.' | b'-'))
        });
    (is_digest || is_tag)
        .then_some(value)
        .ok_or(NotificationError::InvalidReference)
}
