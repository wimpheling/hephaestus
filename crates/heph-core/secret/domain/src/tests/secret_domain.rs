use super::{
    AuthorityStatus, DeliveryMode, ExecutionPhase, OpaqueRuntimeCredential, SecretCommandKey,
    SecretDiagnostic, SecretDiagnosticCode, SecretName, SecretSlotKey, SecretStatus,
    SecretUsePolicy, SecretValue, validate_tenant_boundary,
};
use identity_domain::OrganizationId;

const SENTINEL: &str = "sentinel-do-not-disclose-9e25143a";

#[test]
fn validates_bounded_values_and_policies() {
    assert!(SecretName::parse("github_token").is_ok());
    assert!(SecretName::parse("").is_err());
    assert!(SecretName::parse("UPPER").is_err());
    assert!(SecretSlotKey::parse("model-1").is_ok());
    assert!(SecretSlotKey::parse("x".repeat(65)).is_err());

    let policy = SecretUsePolicy {
        delivery_modes: vec![DeliveryMode::Brokered, DeliveryMode::Brokered],
        phases: vec![ExecutionPhase::Normal],
        destinations: vec![String::from("api.example.test")],
    }
    .normalized()
    .expect("policy should validate");
    assert!(policy.permits(
        DeliveryMode::Brokered,
        ExecutionPhase::Normal,
        Some("api.example.test")
    ));
    assert!(!policy.permits(
        DeliveryMode::Raw,
        ExecutionPhase::Normal,
        Some("api.example.test")
    ));
}

#[test]
fn redacts_every_plaintext_representation() {
    let value = SecretValue::new(SENTINEL.as_bytes()).expect("sentinel should be valid");
    let credential =
        OpaqueRuntimeCredential::new(SENTINEL.repeat(2)).expect("credential should be valid");
    let representations = [
        format!("{value:?}"),
        format!("{value}"),
        serde_json::to_string(&value).expect("redacted serialization should work"),
        format!("{credential:?}"),
        format!("{credential}"),
    ];
    for representation in representations {
        assert!(!representation.contains(SENTINEL));
    }
    assert_eq!(value.len(), SENTINEL.len());
    assert_ne!(credential.storage_hash(), [0; 32]);
}

#[test]
fn errors_and_diagnostics_never_include_rejected_values() {
    let error = SecretName::parse(SENTINEL.to_uppercase()).expect_err("uppercase must fail");
    let diagnostic = SecretDiagnostic {
        code: SecretDiagnosticCode::Unauthorized,
        slot: Some(SecretSlotKey::parse("model").expect("slot should validate")),
        message: String::from("runtime is not authorized for this slot"),
    };
    for representation in [
        format!("{error:?}"),
        error.to_string(),
        format!("{diagnostic:?}"),
        serde_json::to_string(&diagnostic).expect("diagnostic should serialize"),
    ] {
        assert!(!representation.contains(SENTINEL));
    }
}

#[test]
fn lifecycle_and_tenant_boundaries_fail_closed() {
    assert!(SecretStatus::Active.can_transition_to(SecretStatus::Disabled));
    assert!(SecretStatus::Disabled.can_transition_to(SecretStatus::Active));
    assert!(!SecretStatus::Purged.can_transition_to(SecretStatus::Active));
    assert_ne!(AuthorityStatus::Active, AuthorityStatus::Revoked);

    let organization = OrganizationId::new();
    assert!(validate_tenant_boundary(organization, organization).is_ok());
    assert!(validate_tenant_boundary(organization, OrganizationId::new()).is_err());
}

#[test]
fn deterministic_command_keys_are_typed_and_unambiguous() {
    let first = SecretCommandKey::derive("rotate", &[b"ab", b"c"]);
    let repeated = SecretCommandKey::derive("rotate", &[b"ab", b"c"]);
    let ambiguous_without_lengths = SecretCommandKey::derive("rotate", &[b"a", b"bc"]);
    let other_operation = SecretCommandKey::derive("create", &[b"ab", b"c"]);
    assert_eq!(first, repeated);
    assert_ne!(first, ambiguous_without_lengths);
    assert_ne!(first, other_operation);
}

#[test]
fn identifiers_parse_and_serialize() {
    let id = super::SecretId::new();
    let text = id.to_string();
    assert_eq!(
        text.parse::<super::SecretId>().expect("UUID should parse"),
        id
    );
    assert_eq!(
        serde_json::from_str::<super::SecretId>(
            &serde_json::to_string(&id).expect("identifier should serialize")
        )
        .expect("identifier should deserialize"),
        id
    );
    assert!("not-a-uuid".parse::<super::SecretId>().is_err());
}
