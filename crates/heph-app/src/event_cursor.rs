//! Signed, versioned, scope-bound application-event cursor tokens.

// This private module's crate-visible codec is shared by its two adapters.
#![allow(clippy::redundant_pub_crate)]

use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use uuid::Uuid;

const VERSION: &str = "v1";
const DOMAIN: &[u8] = b"hephaestus-product-event-cursor-v1\0";

#[derive(Clone)]
pub(crate) struct EventCursorCodec {
    key: [u8; 32],
}

impl EventCursorCodec {
    pub(crate) const fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    pub(crate) fn encode(&self, scope_kind: &str, scope_id: Uuid, cursor: i64) -> String {
        let payload = format!("{VERSION}.{scope_kind}.{scope_id}.{cursor}");
        let signature = self.signature(payload.as_bytes());
        format!("{payload}.{}", hex(&signature))
    }

    pub(crate) fn decode(
        &self,
        token: &str,
        expected_scope_kind: &str,
        expected_scope_id: Uuid,
    ) -> Option<i64> {
        if token.len() > 192 {
            return None;
        }
        let (payload, signature) = token.rsplit_once('.')?;
        let mut parts = payload.split('.');
        let version = parts.next()?;
        let scope_kind = parts.next()?;
        let scope_id = parts.next()?.parse::<Uuid>().ok()?;
        let cursor_text = parts.next()?;
        if parts.next().is_some()
            || version != VERSION
            || scope_kind != expected_scope_kind
            || scope_id != expected_scope_id
        {
            return None;
        }
        let cursor = cursor_text.parse::<i64>().ok()?;
        if cursor < 0 || cursor.to_string() != cursor_text {
            return None;
        }
        let signature = decode_hex(signature)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).ok()?;
        mac.update(DOMAIN);
        mac.update(payload.as_bytes());
        mac.verify_slice(&signature).ok()?;
        Some(cursor)
    }

    fn signature(&self, payload: &[u8]) -> [u8; 32] {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key)
            .expect("SHA-256 HMAC accepts keys of any length");
        mac.update(DOMAIN);
        mac.update(payload);
        mac.finalize().into_bytes().into()
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").expect("writing to String cannot fail");
    }
    result
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64 {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(text, 16).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::EventCursorCodec;
    use uuid::Uuid;

    #[test]
    fn cursor_rejects_tamper_and_cross_scope_replay() {
        let codec = EventCursorCodec::new([7; 32]);
        let repository = Uuid::new_v4();
        let token = codec.encode("repository", repository, 42);
        assert_eq!(codec.decode(&token, "repository", repository), Some(42));
        assert_eq!(codec.decode(&token, "project", repository), None);
        assert_eq!(codec.decode(&token, "repository", Uuid::new_v4()), None);
        let mut tampered = token.into_bytes();
        let last = tampered.last_mut().expect("nonempty token");
        *last = if *last == b'0' { b'1' } else { b'0' };
        assert_eq!(
            codec.decode(
                std::str::from_utf8(&tampered).expect("ASCII token"),
                "repository",
                repository
            ),
            None
        );
    }
}
