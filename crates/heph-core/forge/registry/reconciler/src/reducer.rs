//! Pure reduction of exact Zot observations into lifecycle actions.

use crate::model::{
    ClaimedNotification, Inconsistency, IntentReconciliation, NotificationCompletion,
    NotificationReduction, ReconciliationAction, ZotInspection,
};
use registry_domain::{
    OciDescriptor, PlatformDescriptor, PublicationIntent, PublicationState, SupplyChainEvidence,
    VerifiedPublication,
};

pub fn orphan_reduction(claim: &ClaimedNotification) -> NotificationReduction {
    NotificationReduction {
        notification_id: claim.id,
        completion: NotificationCompletion::Rejected {
            failure_code: "unknown_namespace".to_owned(),
        },
        intents: Vec::new(),
        actions: vec![ReconciliationAction::OrphanNamespace {
            repository_path: claim.repository_path.clone(),
        }],
    }
}

pub fn reduce_intent(
    intent: &PublicationIntent,
    inspection: ZotInspection,
) -> IntentReconciliation {
    let mut actions = Vec::new();
    match inspection {
        ZotInspection::Missing => {
            if intent.state() == PublicationState::Approved {
                actions.push(ReconciliationAction::MarkMissing {
                    intent_id: intent.id(),
                    reason: Inconsistency::ContentMissing,
                });
            }
        }
        ZotInspection::Invalid => {
            inconsistent(intent, Inconsistency::InvalidZotGraph, &mut actions);
        }
        ZotInspection::Present {
            manifest,
            platforms,
            evidence,
        } => reduce_present(intent, manifest, platforms, evidence, &mut actions),
    }
    IntentReconciliation {
        intent_id: intent.id(),
        state: intent.state(),
        actions,
    }
}

fn reduce_present(
    intent: &PublicationIntent,
    manifest: OciDescriptor,
    platforms: Vec<PlatformDescriptor>,
    evidence: SupplyChainEvidence,
    actions: &mut Vec<ReconciliationAction>,
) {
    let Ok(verification) =
        VerifiedPublication::new(intent.reference(), manifest, platforms, evidence)
    else {
        inconsistent(intent, Inconsistency::InvalidZotGraph, actions);
        return;
    };
    if verification.manifest() != intent.expected_manifest() {
        inconsistent(intent, Inconsistency::ManifestDescriptorMismatch, actions);
        return;
    }
    match intent.state() {
        PublicationState::Pending | PublicationState::Publishing => {
            match intent.clone().record_verified(verification.clone()) {
                Ok(_) => actions.push(ReconciliationAction::RecordVerified {
                    intent_id: intent.id(),
                    verification,
                }),
                Err(_) => inconsistent(intent, Inconsistency::SupplyChainPolicyViolation, actions),
            }
        }
        PublicationState::Verified | PublicationState::Approved => {
            if intent.verification() != Some(&verification) {
                inconsistent(intent, Inconsistency::ImmutableEvidenceMismatch, actions);
            }
        }
        PublicationState::Missing => match intent.clone().restore_verified(&verification) {
            Ok(_) => actions.push(ReconciliationAction::RestoreVerified {
                intent_id: intent.id(),
                verification,
            }),
            Err(_) => inconsistent(intent, Inconsistency::ImmutableEvidenceMismatch, actions),
        },
        PublicationState::Retired => {}
    }
}

fn inconsistent(
    intent: &PublicationIntent,
    reason: Inconsistency,
    actions: &mut Vec<ReconciliationAction>,
) {
    match intent.state() {
        PublicationState::Approved => actions.push(ReconciliationAction::MarkMissing {
            intent_id: intent.id(),
            reason,
        }),
        PublicationState::Missing => {}
        PublicationState::Pending
        | PublicationState::Publishing
        | PublicationState::Verified
        | PublicationState::Retired => actions.push(ReconciliationAction::Investigate {
            intent_id: intent.id(),
            reason,
        }),
    }
}
