//! Controlled comparison tests; no `PostgreSQL` admission or native proof.
use super::version;
use crate::owned_ready_retention::context::configured::receipt;
use recipe_application::{DeploymentError, DeploymentLifecycle};
use release_domain::ContentHash;

#[test]
fn prior_receipt_and_current_association_have_distinct_versions() {
    let input = ContentHash::digest(b"original Remove input at version 12");
    assert_eq!(
        version(DeploymentLifecycle::Removing, 18, 12, input, Some(input)).unwrap(),
        18
    );
    assert_eq!(
        version(DeploymentLifecycle::Removing, 18, 12, input, None).unwrap(),
        12
    );
    assert_eq!(
        input,
        ContentHash::digest(b"original Remove input at version 12")
    );
}

#[test]
fn recovery_receipt_requires_exact_configured_original() {
    let input = ContentHash::digest(b"original Recovery Remove input");
    assert!(matches!(
        receipt("recovery_required", input.as_bytes(), None),
        Err(DeploymentError::InputConflict)
    ));
    receipt("recovery_required", input.as_bytes(), Some(input)).unwrap();
    assert!(matches!(
        receipt(
            "recovery_required",
            ContentHash::digest(b"changed").as_bytes(),
            Some(input)
        ),
        Err(DeploymentError::InputConflict)
    ));
    receipt("removing", input.as_bytes(), None).unwrap();
    assert!(matches!(
        receipt("removed", input.as_bytes(), Some(input)),
        Err(DeploymentError::InputConflict)
    ));
}

#[test]
fn conflicting_or_unbounded_association_cas_denies() {
    let input = ContentHash::digest(b"original");
    let changed = ContentHash::digest(b"different command input");
    assert_eq!(
        version(
            DeploymentLifecycle::RecoveryRequired,
            19,
            12,
            input,
            Some(input)
        )
        .unwrap(),
        19
    );
    assert!(matches!(
        version(DeploymentLifecycle::Removing, 19, 12, input, Some(changed)),
        Err(DeploymentError::InputConflict)
    ));
    assert!(matches!(
        version(DeploymentLifecycle::Removed, 19, 12, input, Some(input)),
        Err(DeploymentError::InputConflict)
    ));
    assert!(matches!(
        version(DeploymentLifecycle::Removing, 0, 12, input, Some(input)),
        Err(DeploymentError::InputConflict)
    ));
    assert!(matches!(
        version(
            DeploymentLifecycle::Removing,
            u64::MAX,
            12,
            input,
            Some(input)
        ),
        Err(DeploymentError::StaleClaim)
    ));
}
