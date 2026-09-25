use super::support::*;
use crate::{
    OciMediaType, PlatformDescriptor, PublicationLifecycleError, PublicationState,
    RegistryConsumptionError, RegistryValueError, SupplyChainEvidence, SupplyChainReferrer,
    SupplyChainReferrerKind, VerifiedPublication,
};

#[test]
fn approvals_are_immutable_and_transition_idempotently() {
    let pending = intent();
    let publishing = pending.begin_publishing().expect("publishing");
    let retried = publishing.retry().expect("retry");
    let publishing = retried.begin_publishing().expect("publishing");
    let verified = verification(publishing.reference());
    let approved = publishing
        .record_verified(verified.clone())
        .expect("verified")
        .approve()
        .expect("approved");
    let repeated = approved
        .clone()
        .record_verified(verified)
        .expect("same verification")
        .approve()
        .expect("same approval");
    assert_eq!(repeated, approved);
    assert_eq!(approved.approved_reference(), Ok(approved.reference()));

    let conflicting = VerifiedPublication::new(
        approved.reference(),
        descriptor(A, OciMediaType::IMAGE_INDEX),
        vec![
            PlatformDescriptor::new(
                descriptor('f', OciMediaType::IMAGE_MANIFEST),
                "linux",
                "arm64",
                None,
            )
            .expect("platform"),
        ],
        approved
            .verification()
            .expect("verification")
            .evidence()
            .clone(),
    )
    .expect("valid but different verification");
    assert_eq!(
        approved.record_verified(conflicting),
        Err(PublicationLifecycleError::ConflictingVerification)
    );
}

#[test]
fn missing_content_fails_closed_and_requires_exact_reverification() {
    let publishing = intent().begin_publishing().expect("publishing");
    let verified = verification(publishing.reference());
    let approved = publishing
        .record_verified(verified.clone())
        .expect("verified")
        .approve()
        .expect("approved");
    let missing = approved.mark_missing().expect("missing");
    assert_eq!(
        missing.approved_reference(),
        Err(RegistryConsumptionError::MissingContent)
    );
    let restored = missing.restore_verified(&verified).expect("restored");
    assert_eq!(restored.state(), PublicationState::Approved);

    let retired = restored.retire();
    assert_eq!(retired.state(), PublicationState::Retired);
    assert_eq!(retired.clone().retire(), retired);
    assert_eq!(
        retired.approved_reference(),
        Err(RegistryConsumptionError::Retired)
    );
}

#[test]
fn verification_requires_all_policy_evidence_and_matching_subjects() {
    let reference = reference();
    let subject = reference.digest().clone();
    let incomplete = SupplyChainEvidence::new(
        subject.clone(),
        vec![SupplyChainReferrer::new(
            SupplyChainReferrerKind::Sbom,
            subject.clone(),
            descriptor(B, "application/spdx+json"),
            media_type("application/spdx+json"),
        )],
    )
    .expect("well-formed but incomplete evidence");
    let verification = VerifiedPublication::new(
        &reference,
        descriptor(A, OciMediaType::IMAGE_INDEX),
        vec![
            PlatformDescriptor::new(
                descriptor(E, OciMediaType::IMAGE_MANIFEST),
                "linux",
                "amd64",
                None,
            )
            .expect("platform"),
        ],
        incomplete,
    )
    .expect("structurally verified");
    assert_eq!(
        intent().record_verified(verification),
        Err(PublicationLifecycleError::InvalidValue(
            RegistryValueError::MissingRequiredReferrer
        ))
    );

    let wrong_subject = SupplyChainEvidence::new(
        subject,
        vec![SupplyChainReferrer::new(
            SupplyChainReferrerKind::Sbom,
            digest(B),
            descriptor(C, "application/spdx+json"),
            media_type("application/spdx+json"),
        )],
    );
    assert_eq!(wrong_subject, Err(RegistryValueError::InvalidReferrer));
}
