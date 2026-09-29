use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey};
use std::str::FromStr;

use crate::{
    MAX_KEY_ID_LENGTH, MAX_TOKEN_LIFETIME_SECONDS, MIN_HMAC_SECRET_LENGTH, RegistryTokenError,
};

/// A non-secret key identifier carried in the JWT `kid` header.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyId(String);

impl KeyId {
    /// Returns the key identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for KeyId {
    type Err = RegistryTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || value.len() > MAX_KEY_ID_LENGTH
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(RegistryTokenError::InvalidKeyId);
        }
        Ok(Self(value.to_owned()))
    }
}

/// A bounded short-lived token duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenLifetime(pub(crate) u64);

impl TokenLifetime {
    /// Creates a duration from one second through fifteen minutes.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or excessively long lifetimes.
    pub const fn new(seconds: u64) -> Result<Self, RegistryTokenError> {
        if seconds == 0 || seconds > MAX_TOKEN_LIFETIME_SECONDS {
            Err(RegistryTokenError::InvalidLifetime)
        } else {
            Ok(Self(seconds))
        }
    }

    /// Returns the duration in seconds.
    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

/// A Unix timestamp supplied by the transport or runtime clock adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnixTimestamp(pub(crate) u64);

impl UnixTimestamp {
    /// Creates a timestamp from whole Unix seconds.
    #[must_use]
    pub const fn new(seconds: u64) -> Self {
        Self(seconds)
    }

    /// Returns whole Unix seconds.
    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

/// HMAC-SHA256 signing material held by the secret/runtime adapter.
pub struct SigningKey {
    pub(crate) key_id: KeyId,
    pub(crate) algorithm: Algorithm,
    pub(crate) key: EncodingKey,
}

impl SigningKey {
    /// Creates HMAC-SHA256 signing material.
    ///
    /// # Errors
    ///
    /// Returns an error when the supplied key material is too short.
    pub fn hs256(key_id: KeyId, secret: &[u8]) -> Result<Self, RegistryTokenError> {
        if secret.len() < MIN_HMAC_SECRET_LENGTH {
            return Err(RegistryTokenError::InsufficientKeyMaterial);
        }
        Ok(Self {
            key_id,
            algorithm: Algorithm::HS256,
            key: EncodingKey::from_secret(secret),
        })
    }

    /// Creates RSA-SHA256 signing material from a private PEM key.
    ///
    /// # Errors
    ///
    /// Returns an error when the PEM does not contain a usable RSA private key.
    pub fn rs256_pem(key_id: KeyId, private_key_pem: &[u8]) -> Result<Self, RegistryTokenError> {
        let key = EncodingKey::from_rsa_pem(private_key_pem)
            .map_err(RegistryTokenError::InvalidKeyMaterial)?;
        Ok(Self {
            key_id,
            algorithm: Algorithm::RS256,
            key,
        })
    }
}

/// HMAC-SHA256 verification material that can overlap during key rotation.
pub struct VerificationKey {
    pub(crate) key_id: KeyId,
    pub(crate) algorithm: Algorithm,
    pub(crate) key: DecodingKey,
}

impl VerificationKey {
    /// Creates HMAC-SHA256 verification material.
    ///
    /// # Errors
    ///
    /// Returns an error when the supplied key material is too short.
    pub fn hs256(key_id: KeyId, secret: &[u8]) -> Result<Self, RegistryTokenError> {
        if secret.len() < MIN_HMAC_SECRET_LENGTH {
            return Err(RegistryTokenError::InsufficientKeyMaterial);
        }
        Ok(Self {
            key_id,
            algorithm: Algorithm::HS256,
            key: DecodingKey::from_secret(secret),
        })
    }

    /// Creates RSA-SHA256 verification material from a public PEM key or
    /// certificate accepted by `jsonwebtoken`.
    ///
    /// # Errors
    ///
    /// Returns an error when the PEM does not contain usable RSA public key
    /// material.
    pub fn rs256_pem(key_id: KeyId, public_key_pem: &[u8]) -> Result<Self, RegistryTokenError> {
        let key = DecodingKey::from_rsa_pem(public_key_pem)
            .map_err(RegistryTokenError::InvalidKeyMaterial)?;
        Ok(Self {
            key_id,
            algorithm: Algorithm::RS256,
            key,
        })
    }
}
