//! Pure, bounded canonical record validation; parsing performs no host actions.

use super::JournalError;
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 8] = b"HEPHOWN1";
pub const MAX_RECORD_BYTES: usize = 16 * 1024;
const HEADER_BYTES: usize = 13;
const HASH_BYTES: usize = 32;

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Kind {
    Birth = 1,
    Attempt = 2,
    Inode = 3,
    Allocated = 4,
    Published = 5,
    FormatIntent = 6,
    AllocationIntent = 7,
}

pub fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn encode(kind: Kind, payload: &[u8]) -> Result<Vec<u8>, JournalError> {
    if payload.len() > MAX_RECORD_BYTES - HEADER_BYTES - HASH_BYTES {
        return Err(JournalError::RecoveryRequired(
            "journal record exceeds its bound",
        ));
    }
    let mut bytes = Vec::with_capacity(HEADER_BYTES + payload.len() + HASH_BYTES);
    bytes.extend_from_slice(MAGIC);
    bytes.push(kind as u8);
    bytes.extend_from_slice(
        &u32::try_from(payload.len())
            .map_err(|_| JournalError::RecoveryRequired("journal length overflows"))?
            .to_be_bytes(),
    );
    bytes.extend_from_slice(payload);
    let hash = digest(&bytes);
    bytes.extend_from_slice(&hash);
    Ok(bytes)
}

pub fn decode(kind: Kind, bytes: &[u8]) -> Result<&[u8], JournalError> {
    if !(HEADER_BYTES + HASH_BYTES..=MAX_RECORD_BYTES).contains(&bytes.len())
        || bytes.get(..8) != Some(MAGIC.as_slice())
        || bytes.get(8) != Some(&(kind as u8))
    {
        return Err(JournalError::RecoveryRequired(
            "journal header is incomplete or unknown",
        ));
    }
    let length = u32::from_be_bytes(
        bytes[9..13]
            .try_into()
            .map_err(|_| JournalError::RecoveryRequired("journal length is incomplete"))?,
    );
    let length = usize::try_from(length)
        .map_err(|_| JournalError::RecoveryRequired("journal length overflows"))?;
    if length > MAX_RECORD_BYTES - HEADER_BYTES - HASH_BYTES {
        return Err(JournalError::RecoveryRequired(
            "journal payload exceeds its bound",
        ));
    }
    if bytes.len() != HEADER_BYTES + length + HASH_BYTES {
        return Err(JournalError::RecoveryRequired(
            "journal record is truncated or has trailing bytes",
        ));
    }
    let end = HEADER_BYTES + length;
    if bytes[end..] != digest(&bytes[..end]) {
        return Err(JournalError::RecoveryRequired(
            "journal record checksum differs",
        ));
    }
    Ok(&bytes[HEADER_BYTES..end])
}

pub struct Cursor<'a> {
    bytes: &'a [u8],
}
impl<'a> Cursor<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    pub const fn take(&mut self, count: usize) -> Result<&'a [u8], JournalError> {
        if count > self.bytes.len() {
            return Err(JournalError::RecoveryRequired(
                "journal payload is truncated",
            ));
        }
        let (value, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(value)
    }
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], JournalError> {
        self.take(N)?
            .try_into()
            .map_err(|_| JournalError::RecoveryRequired("journal field length differs"))
    }
    pub const fn finish(self) -> Result<(), JournalError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(JournalError::RecoveryRequired(
                "journal payload has unknown fields",
            ))
        }
    }
}
