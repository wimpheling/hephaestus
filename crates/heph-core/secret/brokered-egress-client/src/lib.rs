//! Released, guest-safe client for the bounded brokered-egress protocol.
//!
//! This crate deliberately contains no host transport, rule, or secret
//! substitution implementation. A released workload supplies only its private
//! stream and opaque runtime credential; it can never acquire an IP path or
//! observe plaintext secret material.

use runtime_types::RunId;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// Maximum length of one broker wire frame.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// One bounded request sent from a released guest to its host broker.
///
/// The credential intentionally omits formatting traits so accidental debug
/// logging cannot disclose the short-lived bearer bytes.
#[derive(Deserialize, Serialize)]
pub struct WireBrokerRequest {
    /// Short-lived credential read from the guest runtime-authority file.
    pub credential: Vec<u8>,
    /// Exact claimed runtime session.
    pub run_id: RunId,
    /// Symbolic released secret slot.
    pub slot: String,
    /// Exact allowlisted destination hostname.
    pub destination: String,
    /// Semantic adapter operation.
    pub operation: String,
    /// Bounded, non-secret request body.
    pub body: Vec<u8>,
}

/// Sanitized provider-neutral broker response.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WireBrokerResponse {
    /// Stable result class without internal diagnostics.
    pub status: WireBrokerStatus,
    /// Adapter-sanitized body only.
    pub body: Vec<u8>,
}

/// Stable wire result class without internal diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireBrokerStatus {
    /// Semantic operation completed.
    Succeeded,
    /// Authority or operation was denied.
    Denied,
    /// A retry may succeed.
    Retryable,
}

/// Guest-side bounded client for a private broker stream.
pub struct BrokeredHttpsClient<S> {
    stream: S,
}

impl<S: Read + Write> BrokeredHttpsClient<S> {
    /// Creates a client over an already-connected private broker stream.
    #[must_use]
    pub const fn new(stream: S) -> Self {
        Self { stream }
    }

    /// Sends one bounded request and returns its sanitized broker response.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for stream failure, malformed response JSON, or a
    /// frame outside the protocol bound.
    pub fn call(&mut self, request: &WireBrokerRequest) -> io::Result<WireBrokerResponse> {
        let payload = serde_json::to_vec(request)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        write_frame(&mut self.stream, &payload)?;
        let payload = read_frame(&mut self.stream)?;
        serde_json::from_slice(&payload)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

fn write_frame(stream: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "broker frame is oversized"))?;
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "broker frame is outside the protocol bound",
        ));
    }
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(payload)?;
    stream.flush()
}

fn read_frame(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length)?;
    let length = usize::try_from(u32::from_be_bytes(length))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid broker frame length"))?;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "broker frame is outside the protocol bound",
        ));
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload)?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn client_rejects_empty_response_frame() {
        let mut bytes = Cursor::new(0_u32.to_be_bytes().to_vec());
        assert!(read_frame(&mut bytes).is_err());
    }
}
