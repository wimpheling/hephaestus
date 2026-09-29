use super::{CapabilityAuditError, CapabilityAuditPage, CapabilityAuditReason};

#[test]
fn reason_codes_are_bounded_machine_values() {
    assert_eq!(
        CapabilityAuditReason::parse("live_authorization_revoked")
            .expect("canonical reason")
            .as_str(),
        "live_authorization_revoked"
    );
    for invalid in ["", "ContainsSecret", "has-hyphen", "has space"] {
        assert_eq!(
            CapabilityAuditReason::parse(invalid),
            Err(CapabilityAuditError::InvalidReasonCode)
        );
    }
    assert_eq!(
        CapabilityAuditReason::parse(format!("a{}", "b".repeat(64))),
        Err(CapabilityAuditError::InvalidReasonCode)
    );
}

#[test]
fn inspection_pages_are_bounded() {
    assert_eq!(
        CapabilityAuditPage::new(0, None),
        Err(CapabilityAuditError::InvalidPageSize)
    );
    assert!(CapabilityAuditPage::new(200, None).is_ok());
    assert_eq!(
        CapabilityAuditPage::new(201, None),
        Err(CapabilityAuditError::InvalidPageSize)
    );
}
