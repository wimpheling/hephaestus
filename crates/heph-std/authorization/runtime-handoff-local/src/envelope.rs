//! Strict terminal envelope shape without changing legacy live decoding.

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpiredEnvelope {
    pub(super) algorithm: String,
    pub(super) nonce: [u8; 12],
    pub(super) ciphertext: Vec<u8>,
    pub(super) expires_at_unix_nanos: i128,
}
