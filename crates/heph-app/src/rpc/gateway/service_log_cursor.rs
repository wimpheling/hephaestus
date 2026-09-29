//! Scope-bound cursors for the service-log query.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gateway_edge::{GatewayServiceLogReadCursor, GatewayServiceLogReadScope};
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use std::fmt;
use uuid::Uuid;

const VERSION: u8 = 1;
const DOMAIN: &[u8] = b"hephaestus-gateway-service-log-cursor-v1\0";
const PAYLOAD_BYTES: usize = 1 + (4 * 16) + 8 + 8;
const TOKEN_BYTES: usize = PAYLOAD_BYTES + 32;
const MAX_TOKEN_LENGTH: usize = 192;

/// A cursor codec with a key that is never exposed through formatting.
#[derive(Clone)]
pub(super) struct ServiceLogCursorCodec {
    key: [u8; 32],
}

impl fmt::Debug for ServiceLogCursorCodec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ServiceLogCursorCodec { key: [redacted] }")
    }
}

impl ServiceLogCursorCodec {
    pub(super) const fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    pub(super) fn encode(&self, cursor: GatewayServiceLogReadCursor) -> String {
        let scope = cursor.scope();
        let mut payload = [0_u8; PAYLOAD_BYTES];
        payload[0] = VERSION;
        let mut offset = 1;
        for id in [
            scope.project_id,
            scope.gateway_id,
            scope.revision_id,
            scope.instance_id,
        ] {
            payload[offset..offset + 16].copy_from_slice(id.as_bytes());
            offset += 16;
        }
        payload[offset..offset + 8].copy_from_slice(&scope.fencing_token.to_be_bytes());
        offset += 8;
        payload[offset..offset + 8].copy_from_slice(
            &i64::try_from(cursor.sequence())
                .expect("validated cursor sequence fits PostgreSQL bigint")
                .to_be_bytes(),
        );

        let tag = self.tag(&payload);
        let mut token = [0_u8; TOKEN_BYTES];
        token[..PAYLOAD_BYTES].copy_from_slice(&payload);
        token[PAYLOAD_BYTES..].copy_from_slice(&tag);
        URL_SAFE_NO_PAD.encode(token)
    }

    pub(super) fn decode(
        &self,
        token: &str,
        expected_scope: GatewayServiceLogReadScope,
    ) -> Result<GatewayServiceLogReadCursor, CursorError> {
        if token.is_empty() || token.len() > MAX_TOKEN_LENGTH || !token.is_ascii() {
            return Err(CursorError::Malformed);
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|_| CursorError::Malformed)?;
        if decoded.len() != TOKEN_BYTES {
            return Err(CursorError::Malformed);
        }
        if URL_SAFE_NO_PAD.encode(&decoded) != token {
            return Err(CursorError::Malformed);
        }
        let payload = &decoded[..PAYLOAD_BYTES];
        let supplied_tag = &decoded[PAYLOAD_BYTES..];
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key)
            .expect("SHA-256 HMAC accepts keys of any length");
        mac.update(DOMAIN);
        mac.update(payload);
        mac.verify_slice(supplied_tag)
            .map_err(|_| CursorError::InvalidSignature)?;
        if payload[0] != VERSION {
            return Err(CursorError::Malformed);
        }
        let mut offset = 1;
        let project_id = read_uuid(payload, &mut offset)?;
        let gateway_id = read_uuid(payload, &mut offset)?;
        let revision_id = read_uuid(payload, &mut offset)?;
        let instance_id = read_uuid(payload, &mut offset)?;
        let fencing_token = read_i64(payload, &mut offset)?;
        let sequence = read_i64(payload, &mut offset)?;
        if fencing_token <= 0 || sequence < 0 {
            return Err(CursorError::Malformed);
        }
        let scope = GatewayServiceLogReadScope::new(
            project_id,
            gateway_id,
            revision_id,
            instance_id,
            fencing_token,
        )
        .map_err(|_| CursorError::Malformed)?;
        if scope != expected_scope {
            return Err(CursorError::WrongScope);
        }
        GatewayServiceLogReadCursor::new(
            scope,
            u64::try_from(sequence).map_err(|_| CursorError::Malformed)?,
        )
        .map_err(|_| CursorError::Malformed)
    }

    fn tag(&self, payload: &[u8]) -> [u8; 32] {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key)
            .expect("SHA-256 HMAC accepts keys of any length");
        mac.update(DOMAIN);
        mac.update(payload);
        mac.finalize().into_bytes().into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CursorError {
    Malformed,
    InvalidSignature,
    WrongScope,
}

fn read_uuid(payload: &[u8], offset: &mut usize) -> Result<Uuid, CursorError> {
    let bytes = payload
        .get(*offset..*offset + 16)
        .ok_or(CursorError::Malformed)?;
    *offset += 16;
    Ok(Uuid::from_bytes(
        bytes.try_into().map_err(|_| CursorError::Malformed)?,
    ))
}

fn read_i64(payload: &[u8], offset: &mut usize) -> Result<i64, CursorError> {
    let bytes = payload
        .get(*offset..*offset + 8)
        .ok_or(CursorError::Malformed)?;
    *offset += 8;
    Ok(i64::from_be_bytes(
        bytes.try_into().map_err(|_| CursorError::Malformed)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{MAX_TOKEN_LENGTH, PAYLOAD_BYTES, ServiceLogCursorCodec, TOKEN_BYTES};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use gateway_edge::{GatewayServiceLogReadCursor, GatewayServiceLogReadScope};
    use uuid::Uuid;

    fn scope() -> GatewayServiceLogReadScope {
        GatewayServiceLogReadScope::new(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
            i64::MAX,
        )
        .expect("valid scope")
    }

    #[test]
    fn cursor_round_trips_and_is_bounded() {
        let codec = ServiceLogCursorCodec::new([7; 32]);
        let cursor = GatewayServiceLogReadCursor::new(scope(), i64::MAX as u64).unwrap();
        let token = codec.encode(cursor);
        assert!(token.len() <= MAX_TOKEN_LENGTH);
        assert_eq!(codec.decode(&token, scope()).unwrap(), cursor);
        assert!(format!("{codec:?}").contains("redacted"));
    }

    #[test]
    fn cursor_rejects_tamper_key_version_truncation_oversize_and_scope_changes() {
        let codec = ServiceLogCursorCodec::new([7; 32]);
        let cursor = GatewayServiceLogReadCursor::new(scope(), 0).unwrap();
        let token = codec.encode(cursor);
        let mut tampered = token.as_bytes().to_vec();
        let last = tampered.last_mut().unwrap();
        *last = if *last == b'A' { b'B' } else { b'A' };
        assert!(
            codec
                .decode(std::str::from_utf8(&tampered).unwrap(), scope())
                .is_err()
        );
        assert!(
            ServiceLogCursorCodec::new([8; 32])
                .decode(&token, scope())
                .is_err()
        );
        assert!(codec.decode(&token[..token.len() - 1], scope()).is_err());
        assert!(
            codec
                .decode(&"A".repeat(MAX_TOKEN_LENGTH + 1), scope())
                .is_err()
        );
        let mut changed = scope();
        changed.project_id = Uuid::from_u128(22);
        assert!(codec.decode(&token, changed).is_err());
        changed = scope();
        changed.gateway_id = Uuid::from_u128(22);
        assert!(codec.decode(&token, changed).is_err());
        changed = scope();
        changed.revision_id = Uuid::from_u128(22);
        assert!(codec.decode(&token, changed).is_err());
        changed = scope();
        changed.instance_id = Uuid::from_u128(22);
        assert!(codec.decode(&token, changed).is_err());
        changed = scope();
        changed.fencing_token -= 1;
        assert!(codec.decode(&token, changed).is_err());

        assert!(
            signed_mutation(&codec, &token, |payload| payload[0] = 2)
                .and_then(|token| codec.decode(&token, scope()).err())
                .is_some()
        );
        assert!(
            signed_mutation(&codec, &token, |payload| {
                payload[65..73].copy_from_slice(&0_i64.to_be_bytes());
            })
            .and_then(|token| codec.decode(&token, scope()).err())
            .is_some()
        );
        assert!(
            signed_mutation(&codec, &token, |payload| {
                payload[73..81].copy_from_slice(&(-1_i64).to_be_bytes());
            })
            .and_then(|token| codec.decode(&token, scope()).err())
            .is_some()
        );
    }

    fn signed_mutation(
        codec: &ServiceLogCursorCodec,
        token: &str,
        mutate: impl FnOnce(&mut [u8]),
    ) -> Option<String> {
        let mut decoded = URL_SAFE_NO_PAD.decode(token).ok()?;
        mutate(&mut decoded[..PAYLOAD_BYTES]);
        let tag = codec.tag(&decoded[..PAYLOAD_BYTES]);
        decoded[PAYLOAD_BYTES..TOKEN_BYTES].copy_from_slice(&tag);
        Some(URL_SAFE_NO_PAD.encode(decoded))
    }
}
