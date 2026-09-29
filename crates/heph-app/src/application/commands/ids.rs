//! Deterministic command identifiers and idempotency keys.

use identity_domain::AuthenticatedIdentity;
use release_domain::ReleaseCommandKey;
use secret_domain::SecretCommandKey;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(super) fn release_key(
    identity: &AuthenticatedIdentity,
    operation: &str,
    aggregate_id: Uuid,
) -> ReleaseCommandKey {
    ReleaseCommandKey::derive(
        operation,
        &[
            identity.idempotency_id.as_uuid().as_bytes(),
            aggregate_id.as_bytes(),
        ],
    )
}

pub(super) fn secret_key(
    identity: &AuthenticatedIdentity,
    operation: &str,
    aggregate_id: Uuid,
) -> SecretCommandKey {
    SecretCommandKey::derive(
        operation,
        &[
            identity.idempotency_id.as_uuid().as_bytes(),
            aggregate_id.as_bytes(),
        ],
    )
}

pub(super) fn stable_id(identity: &AuthenticatedIdentity, purpose: &str) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus-command-resource-v1\0");
    digest.update(identity.user_id.as_uuid().as_bytes());
    digest.update(identity.idempotency_id.as_uuid().as_bytes());
    digest.update(purpose.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.finalize()[..16]);
    // RFC 9562 version 8 is reserved for application-defined UUID layouts.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

pub(super) fn declared_rule_id(
    identity: &AuthenticatedIdentity,
    requested_rule_id: Option<Uuid>,
) -> Uuid {
    requested_rule_id.unwrap_or_else(|| stable_id(identity, "declare_brokered_https_rule.rule"))
}
#[cfg(test)]
mod tests {
    use super::{declared_rule_id, stable_id};
    use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn retry_resource_ids_are_stable_and_operation_scoped() {
        let identity = AuthenticatedIdentity::new(
            UserId::new(),
            "test",
            "subject",
            json!({}),
            RequestId::new(),
        );
        let first = stable_id(&identity, "create_secret.secret");
        assert_eq!(first, stable_id(&identity, "create_secret.secret"));
        assert_ne!(first, stable_id(&identity, "create_secret.version"));
        assert_eq!(first.get_version_num(), 8);
    }

    #[test]
    fn requested_brokered_rule_id_is_preserved_and_absent_uses_stable_id() {
        let identity = AuthenticatedIdentity::new(
            UserId::new(),
            "test",
            "subject",
            json!({}),
            RequestId::new(),
        );
        let requested = Uuid::new_v4();
        assert_eq!(declared_rule_id(&identity, Some(requested)), requested);
        assert_eq!(
            declared_rule_id(&identity, None),
            stable_id(&identity, "declare_brokered_https_rule.rule")
        );
    }
}
