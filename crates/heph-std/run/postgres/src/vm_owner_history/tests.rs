//! Pure classification denials; these records are not genuine producer fixtures.

use super::records::{Original, classify, namespace};
use run_domain::{VmOwnerHistory, VmOwnerHistoryUnknown};
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;

fn expected() -> VmProviderOwnerScope {
    VmProviderOwnerScope::new(Uuid::new_v4().to_string(), "configured-host".into()).unwrap()
}

#[test]
fn unqualified_foreign_row_cannot_disappear_behind_host_filter() {
    let expected = expected();
    let row = Original {
        execution_profile: "owned_canonical".into(),
        namespace: Some(expected.namespace().into()),
        host: Some("other-host".into()),
        qualified: false,
    };
    assert_eq!(
        classify(&[row], &expected),
        VmOwnerHistory::Unknown(VmOwnerHistoryUnknown::UnqualifiedOriginal)
    );
}

#[test]
fn historical_owner_text_never_adopts_historical_original() {
    let expected = expected();
    let row = Original {
        execution_profile: "historical_unproven".into(),
        namespace: Some(expected.namespace().into()),
        host: Some(expected.host_id().into()),
        qualified: true,
    };
    assert_eq!(
        classify(&[row], &expected),
        VmOwnerHistory::Unknown(VmOwnerHistoryUnknown::HistoricalUnproven)
    );
}

#[test]
fn nil_noncanonical_and_missing_owner_deny_attribution() {
    let expected = expected();
    for value in [
        None,
        Some(Uuid::nil().to_string()),
        Some("arbitrary-owner".into()),
    ] {
        let row = Original {
            execution_profile: "owned_canonical".into(),
            namespace: value,
            host: Some(expected.host_id().into()),
            qualified: true,
        };
        assert_eq!(
            classify(&[row], &expected),
            VmOwnerHistory::Unknown(VmOwnerHistoryUnknown::MissingOwnerIdentity)
        );
    }
    assert!(namespace("00000000-0000-0000-0000-000000000000").is_err());
    assert!(namespace("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA").is_err());
}
