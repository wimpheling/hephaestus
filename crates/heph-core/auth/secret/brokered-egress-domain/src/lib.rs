//! Typed, provider-neutral contracts for HTTPS placeholder substitution.
//!
//! These values contain authority metadata only.  They intentionally cannot
//! hold secret material, arbitrary URLs, request bodies, or provider schemas.

#[path = "brokered_egress/errors.rs"]
mod errors;
#[path = "brokered_egress/rules.rs"]
mod rules;
#[path = "brokered_egress/types.rs"]
mod types;

/// Prefix of a stable VM-visible placeholder.
pub const PLACEHOLDER_PREFIX: &str = "heph-placeholder:v1:";
/// Maximum normalized header-name length.
pub const MAX_HEADER_NAME_BYTES: usize = 64;
/// Maximum ASCII prefix bytes allowed before an outbound placeholder.
pub const MAX_HEADER_PREFIX_BYTES: usize = 256;

pub use errors::BrokeredEgressError;
pub use rules::{BrokeredSecretRule, HttpInjectionLocation, InjectionDirection};
pub use types::{BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, PlaceholderId};

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn outbound() -> BrokeredSecretRule {
        BrokeredSecretRule {
            id: BrokeredSecretRuleId::from_uuid(Uuid::nil()),
            binding_id: Uuid::new_v4(),
            instance_revision_id: Uuid::new_v4(),
            secret_version_id: Uuid::new_v4(),
            destination: Some(
                ExactHttpsOrigin::parse("https://API.Example.test:443").expect("origin"),
            ),
            location: HttpInjectionLocation::OutboundHeaderPrefix {
                header: HeaderName::parse("Authorization").expect("header"),
                prefix: String::from("Bearer "),
            },
            gateway_route_id: None,
        }
    }

    #[test]
    fn normalizes_exact_origins_and_headers() {
        let rule = outbound().normalized().expect("rule");
        assert_eq!(
            rule.destination.as_ref().expect("destination").as_str(),
            "https://api.example.test"
        );
        assert_eq!(
            rule.placeholder(),
            "heph-placeholder:v1:00000000-0000-0000-0000-000000000000"
        );
        assert_eq!(
            HeaderName::parse("X-Signature").expect("header").as_str(),
            "x-signature"
        );
    }

    #[test]
    fn rejects_broad_or_ambiguous_rules() {
        for origin in [
            "http://api.example.test",
            "https://*.example.test",
            "https://127.0.0.1",
            "https://api.example.test/path",
            "https://[::1]",
        ] {
            assert!(ExactHttpsOrigin::parse(origin).is_err(), "{origin}");
        }
        let mut rule = outbound();
        rule.gateway_route_id = Some(Uuid::new_v4());
        assert_eq!(
            rule.normalized(),
            Err(BrokeredEgressError::AmbiguousRuleScope)
        );
        let inbound = BrokeredSecretRule {
            destination: None,
            location: HttpInjectionLocation::InboundGatewayHeader {
                header: HeaderName::parse("X-Hook-Secret").expect("header"),
            },
            gateway_route_id: Some(Uuid::new_v4()),
            ..outbound()
        };
        assert!(inbound.normalized().is_ok());
    }

    #[test]
    fn rule_hash_is_secret_free_and_stable() {
        let rule = outbound().normalized().expect("rule");
        assert_eq!(rule.normalized_hash(), rule.normalized_hash());
        let serialized = serde_json::to_string(&rule).expect("serialize");
        assert!(!serialized.contains("secret value"));
        assert!(!serialized.contains("heph-placeholder"));
    }
}
