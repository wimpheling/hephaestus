//! Callback credential verification.

use crate::parser::NotificationError;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::fmt;

const CALLBACK_CREDENTIAL_MINIMUM_LENGTH: usize = 43;
const CALLBACK_CREDENTIAL_MAXIMUM_LENGTH: usize = 128;
const CALLBACK_CREDENTIAL_CONTEXT: &[u8] = b"hephaestus/zot-notification-callback/v1";
type HmacSha256 = Hmac<Sha256>;

/// A private callback credential configured in Zot's HTTP event sink.
///
/// The constructor accepts only an unpadded URL-safe base64 token encoding at
/// least 256 bits of entropy. The value is immediately reduced to a verifier;
/// neither the original credential nor its derived verifier are exposed by
/// [`Debug`](fmt::Debug).
#[derive(Clone, PartialEq, Eq)]
pub struct CallbackCredential([u8; 32]);

impl CallbackCredential {
    /// Parses a generated private callback token.
    ///
    /// # Errors
    ///
    /// Returns [`NotificationError::InvalidCallbackCredential`] if the value
    /// cannot safely serve as a high-entropy bearer credential.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, NotificationError> {
        let value = value.as_ref();
        let valid = (CALLBACK_CREDENTIAL_MINIMUM_LENGTH..=CALLBACK_CREDENTIAL_MAXIMUM_LENGTH)
            .contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if !valid {
            return Err(NotificationError::InvalidCallbackCredential);
        }

        Ok(Self(callback_verifier(value.as_bytes())))
    }

    /// Checks a presented token against the derived verifier.
    ///
    /// # Panics
    ///
    /// The fixed context is always a valid HMAC key.
    #[must_use]
    pub fn authenticates(&self, presented: &str) -> bool {
        if !(CALLBACK_CREDENTIAL_MINIMUM_LENGTH..=CALLBACK_CREDENTIAL_MAXIMUM_LENGTH)
            .contains(&presented.len())
            || !presented
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return false;
        }

        // `verify_slice` performs a constant-time comparison. The configured
        // token itself is never retained after construction.
        HmacSha256::new_from_slice(CALLBACK_CREDENTIAL_CONTEXT)
            .expect("the fixed callback credential context is a valid HMAC key")
            .chain_update(presented.as_bytes())
            .verify_slice(&self.0)
            .is_ok()
    }
}

impl fmt::Debug for CallbackCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CallbackCredential([REDACTED])")
    }
}

pub fn callback_verifier(value: &[u8]) -> [u8; 32] {
    HmacSha256::new_from_slice(CALLBACK_CREDENTIAL_CONTEXT)
        .expect("the fixed callback credential context is a valid HMAC key")
        .chain_update(value)
        .finalize()
        .into_bytes()
        .into()
}
