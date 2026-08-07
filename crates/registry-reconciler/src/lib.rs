//! Authoritative reconciliation for forge-owned OCI registry content.
//!
//! Zot event notifications are merely lossy observations: this crate never
//! treats a callback as an approval signal.  It reads Zot again by the exact
//! immutable digest, reduces that observation into typed actions, and leaves
//! lifecycle mutation to a separately authorized executor.

use async_trait::async_trait;
use registry_domain::{
    ImmutableManifestReference, OciDescriptor, PlatformDescriptor, PublicationIntent,
    PublicationIntentId, PublicationState, RegistryNamespace, SupplyChainEvidence,
    VerifiedPublication,
};
use std::time::Duration;

/// Exact target retained from a bounded Zot observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedTarget {
    /// Observed immutable digest.
    pub digest: registry_domain::Sha256Digest,
    /// Observed OCI media type.
    pub media_type: registry_domain::OciMediaType,
}

/// Adapter-neutral claimed notification consumed by reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedNotification {
    /// Durable inbox identity.
    pub id: uuid::Uuid,
    /// Opaque lease ownership token returned unchanged on completion.
    pub lease_token: uuid::Uuid,
    /// Bounded raw path retained when canonical namespace parsing failed.
    pub repository_path: String,
    /// Canonical namespace when the observation addressed one.
    pub namespace: Option<RegistryNamespace>,
    /// Optional exact manifest target from the observation.
    pub target: Option<ObservedTarget>,
}

/// Terminal inbox result committed only after actions succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationCompletion {
    /// The observation was authoritatively reconciled.
    Processed,
    /// The observation was bounded but did not address an owned namespace.
    Rejected {
        /// Stable non-sensitive reason code.
        failure_code: String,
    },
}

/// Opaque failure returned by a reconciliation port.
///
/// The public contract deliberately avoids carrying registry credentials,
/// callback payloads, SQL text, or remote response bodies across layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("registry reconciliation dependency is unavailable")]
pub struct ReconciliationPortError;

/// Durable inbox access required by the notification worker.
#[async_trait]
pub trait NotificationInbox: Send + Sync + 'static {
    /// Claims one notification using the caller's bounded lease duration.
    ///
    /// # Errors
    ///
    /// Returns an opaque storage failure. The observation is left claimed so
    /// it can be retried after its lease expires.
    async fn claim(
        &self,
        lease: Duration,
    ) -> Result<Option<ClaimedNotification>, ReconciliationPortError>;

    /// Marks a claim terminal after its observation was reduced.
    ///
    /// # Errors
    ///
    /// Returns an opaque storage failure when the lease was lost or storage is
    /// unavailable. This method must not mutate publication lifecycle state.
    async fn complete(
        &self,
        claim: &ClaimedNotification,
        completion: NotificationCompletion,
    ) -> Result<(), ReconciliationPortError>;
}

/// Durable source of publication intents for one authoritative pass.
#[async_trait]
pub trait PublicationIntents: Send + Sync + 'static {
    /// Lists every retained intent for a namespace in stable identity order.
    ///
    /// # Errors
    ///
    /// Returns an opaque persistence failure.
    async fn for_namespace(
        &self,
        namespace: &RegistryNamespace,
    ) -> Result<Vec<PublicationIntent>, ReconciliationPortError>;

    /// Lists every retained publication intent in stable identity order.
    ///
    /// This is the missed-event safety net: it runs independently of Zot's
    /// best-effort notification delivery.
    ///
    /// # Errors
    ///
    /// Returns an opaque persistence failure.
    async fn all(&self) -> Result<Vec<PublicationIntent>, ReconciliationPortError>;
}

/// Exact Zot graph read for one immutable digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZotInspection {
    /// Zot no longer serves the requested immutable manifest digest.
    Missing,
    /// Zot served a response for the digest, but its exact descriptors or
    /// referrer graph were malformed, inconsistent, or outside safety bounds.
    Invalid,
    /// Zot returned the manifest, platform descriptors, and referrer subject
    /// graph read back for the requested digest.
    Present {
        /// Top-level manifest or index descriptor.
        manifest: OciDescriptor,
        /// Platform-specific manifests referenced by the top-level content.
        platforms: Vec<PlatformDescriptor>,
        /// Referrers and their declared subject, read from Zot.
        evidence: SupplyChainEvidence,
    },
}

/// Zot read boundary used by notification and scheduled reconciliation.
#[async_trait]
pub trait ZotRegistry: Send + Sync + 'static {
    /// Reads Zot by the exact immutable reference, never by a tag.
    ///
    /// Implementations must obtain both the referrer list and every
    /// referrer's subject from Zot rather than trusting a callback body.
    ///
    /// # Errors
    ///
    /// Returns an opaque transport or Zot service failure. `Missing` is a
    /// successful authoritative response and is represented by
    /// [`ZotInspection::Missing`].
    async fn inspect(
        &self,
        reference: &ImmutableManifestReference,
    ) -> Result<ZotInspection, ReconciliationPortError>;
}

/// Authorized lifecycle executor for reconciliation actions.
///
/// Implementations normally call the registry PostgreSQL adapter, whose
/// transactions append product events through the committed outbox. Diagnostic
/// actions may be recorded or logged, but must never become approval.
#[async_trait]
pub trait ReconciliationActionExecutor: Send + Sync + 'static {
    /// Applies one idempotent action before the originating inbox claim is
    /// completed.
    ///
    /// # Errors
    ///
    /// Returns an opaque persistence failure. The claim remains leased and is
    /// retried after expiry.
    async fn apply(&self, action: &ReconciliationAction) -> Result<(), ReconciliationPortError>;
}

/// Stable semantic reason an immutable Zot graph cannot be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inconsistency {
    /// The exact approved digest is no longer present in Zot.
    ContentMissing,
    /// The graph could not form a valid immutable verification record.
    InvalidZotGraph,
    /// Zot's manifest descriptor differs from the durable expected descriptor.
    ManifestDescriptorMismatch,
    /// Zot's evidence conflicts with already retained immutable evidence.
    ImmutableEvidenceMismatch,
    /// Zot's graph does not satisfy the publication's supply-chain policy.
    SupplyChainPolicyViolation,
}

/// A requested lifecycle mutation for a separately authorized executor.
///
/// There is intentionally no approval variant. A notification or scheduled
/// pass may establish verified evidence, mark an approved digest missing, or
/// propose recovery; policy approval remains an explicit later operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationAction {
    /// Persist newly read immutable evidence as `verified`, never `approved`.
    RecordVerified {
        /// Publication that can record the evidence.
        intent_id: PublicationIntentId,
        /// Exact evidence read back from Zot.
        verification: VerifiedPublication,
    },
    /// Fail closed because a previously approved digest is absent or invalid.
    MarkMissing {
        /// Formerly approved publication.
        intent_id: PublicationIntentId,
        /// Why the exact remote graph is unusable.
        reason: Inconsistency,
    },
    /// Restore a missing approval only when its original evidence is exact.
    RestoreVerified {
        /// Missing publication that can be restored by an executor.
        intent_id: PublicationIntentId,
        /// Exact immutable evidence that permits recovery.
        verification: VerifiedPublication,
    },
    /// Record an observation whose digest/media type does not identify an
    /// existing intent. It is diagnostic only and never changes lifecycle.
    ObservedDifferentTarget {
        /// Namespace containing the observation.
        namespace: RegistryNamespace,
    },
    /// Record a bounded unknown or unclaimed namespace observation.
    OrphanNamespace {
        /// Bounded repository path retained by the notification inbox.
        repository_path: String,
    },
    /// Surface a non-approved inconsistency for investigation without making
    /// an illegal lifecycle transition.
    Investigate {
        /// Publication with inconsistent Zot content.
        intent_id: PublicationIntentId,
        /// Why reconciliation cannot safely verify it.
        reason: Inconsistency,
    },
}

/// Per-intent result of an exact Zot inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentReconciliation {
    /// Durable intent inspected.
    pub intent_id: PublicationIntentId,
    /// State read before any proposed action is executed.
    pub state: PublicationState,
    /// Typed follow-up actions. They have not been applied.
    pub actions: Vec<ReconciliationAction>,
}

/// Reduction result for one claimed Zot notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationReduction {
    /// Claim reduced by this result.
    pub notification_id: uuid::Uuid,
    /// Terminal inbox completion safe to commit after reduction.
    pub completion: NotificationCompletion,
    /// Per-intent authoritative outcomes.
    pub intents: Vec<IntentReconciliation>,
    /// Diagnostic or follow-up actions that have not been applied.
    pub actions: Vec<ReconciliationAction>,
}

/// Result of a scheduled full authoritative reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritativeReconciliation {
    /// Every inspected durable intent.
    pub intents: Vec<IntentReconciliation>,
}

/// Reducer and authoritative reconciliation application service.
pub struct RegistryReconciler<I, P, Z> {
    inbox: I,
    intents: P,
    zot: Z,
}

impl<I, P, Z> RegistryReconciler<I, P, Z> {
    /// Creates the service from explicit storage and Zot ports.
    #[must_use]
    pub const fn new(inbox: I, intents: P, zot: Z) -> Self {
        Self {
            inbox,
            intents,
            zot,
        }
    }
}

impl<I, P, Z> RegistryReconciler<I, P, Z>
where
    I: NotificationInbox,
    P: PublicationIntents,
    Z: ZotRegistry,
{
    /// Claims and reduces at most one notification, then completes its inbox
    /// claim. Publication actions are returned rather than executed.
    ///
    /// # Errors
    ///
    /// Returns when a port is unavailable. In that case the notification is
    /// deliberately not completed and may be retried after its lease expires.
    pub async fn process_next(
        &self,
        lease: Duration,
    ) -> Result<Option<NotificationReduction>, ReconciliationPortError> {
        let Some(claim) = self.inbox.claim(lease).await? else {
            return Ok(None);
        };
        let reduction = self.reduce_claimed(&claim).await?;
        self.inbox
            .complete(&claim, reduction.completion.clone())
            .await?;
        Ok(Some(reduction))
    }

    /// Claims, authoritatively reduces, and applies one observation before
    /// completing its durable inbox claim.
    ///
    /// This is the production path. [`Self::process_next`] remains useful to
    /// consumers that deliberately persist the returned action batch in their
    /// own transaction boundary.
    ///
    /// # Errors
    ///
    /// Returns when a port or action executor is unavailable. No inbox
    /// completion is attempted after an action failure, preserving retry.
    pub async fn process_next_and_apply<E>(
        &self,
        lease: Duration,
        executor: &E,
    ) -> Result<bool, ReconciliationPortError>
    where
        E: ReconciliationActionExecutor,
    {
        let Some(claim) = self.inbox.claim(lease).await? else {
            return Ok(false);
        };
        let reduction = self.reduce_claimed(&claim).await?;
        for intent in &reduction.intents {
            for action in &intent.actions {
                executor.apply(action).await?;
            }
        }
        for action in &reduction.actions {
            executor.apply(action).await?;
        }
        self.inbox.complete(&claim, reduction.completion).await?;
        Ok(true)
    }

    /// Reduces one already claimed notification into unapplied typed actions.
    ///
    /// # Errors
    ///
    /// Returns when durable intents or Zot cannot be read. It does not complete
    /// the claim, allowing the caller to retain retry semantics.
    pub async fn reduce_claimed(
        &self,
        claim: &ClaimedNotification,
    ) -> Result<NotificationReduction, ReconciliationPortError> {
        let Some(namespace) = &claim.namespace else {
            return Ok(orphan_reduction(claim));
        };
        let intents = self.intents.for_namespace(namespace).await?;
        if intents.is_empty() {
            return Ok(orphan_reduction(claim));
        }
        let observed_target_matches = claim.target.as_ref().is_none_or(|target| {
            intents.iter().any(|intent| {
                target.digest == *intent.reference().digest()
                    && target.media_type == *intent.expected_manifest().media_type()
            })
        });
        let mut actions = Vec::new();
        if !observed_target_matches {
            actions.push(ReconciliationAction::ObservedDifferentTarget {
                namespace: namespace.clone(),
            });
        }
        let mut reduced = Vec::with_capacity(intents.len());
        for intent in intents {
            reduced.push(self.reconcile_intent(intent).await?);
        }
        Ok(NotificationReduction {
            notification_id: claim.id,
            completion: NotificationCompletion::Processed,
            intents: reduced,
            actions,
        })
    }

    /// Inspects every durable intent, independent of notifications.
    ///
    /// # Errors
    ///
    /// Returns when durable intents or Zot cannot be read. The caller should
    /// retry the scheduled pass; no lifecycle mutation occurs here.
    pub async fn reconcile_all(
        &self,
    ) -> Result<AuthoritativeReconciliation, ReconciliationPortError> {
        let intents = self.intents.all().await?;
        let mut reduced = Vec::with_capacity(intents.len());
        for intent in intents {
            reduced.push(self.reconcile_intent(intent).await?);
        }
        Ok(AuthoritativeReconciliation { intents: reduced })
    }

    /// Performs a full missed-event reconciliation and applies every proposed
    /// lifecycle action through the authorized executor.
    ///
    /// # Errors
    ///
    /// Returns on the first inspection or action failure. Actions are
    /// idempotent, so the next scheduled pass safely retries the full set.
    pub async fn reconcile_all_and_apply<E>(
        &self,
        executor: &E,
    ) -> Result<AuthoritativeReconciliation, ReconciliationPortError>
    where
        E: ReconciliationActionExecutor,
    {
        let report = self.reconcile_all().await?;
        for intent in &report.intents {
            for action in &intent.actions {
                executor.apply(action).await?;
            }
        }
        Ok(report)
    }

    /// Inspects one intent by its exact immutable Zot digest.
    ///
    /// # Errors
    ///
    /// Returns when Zot cannot be read. A missing digest is returned as a
    /// normal authoritative result and is not an error.
    pub async fn reconcile_intent(
        &self,
        intent: PublicationIntent,
    ) -> Result<IntentReconciliation, ReconciliationPortError> {
        let inspection = self.zot.inspect(intent.reference()).await?;
        Ok(reduce_intent(&intent, inspection))
    }
}

fn orphan_reduction(claim: &ClaimedNotification) -> NotificationReduction {
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

fn reduce_intent(intent: &PublicationIntent, inspection: ZotInspection) -> IntentReconciliation {
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use registry_domain::{
        ImmutableManifestReference, NamespaceClaim, OciMediaType, PlatformImageKey,
        RegistryAuthority, RegistryOwner, Sha256Digest, SupplyChainPolicy, SupplyChainReferrer,
        SupplyChainReferrerKind,
    };
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };
    use uuid::Uuid;

    #[derive(Clone, Default)]
    struct FakeInbox {
        claimed: Arc<Mutex<VecDeque<ClaimedNotification>>>,
        completions: Arc<Mutex<Vec<NotificationCompletion>>>,
    }

    #[async_trait]
    impl NotificationInbox for FakeInbox {
        async fn claim(
            &self,
            _lease: Duration,
        ) -> Result<Option<ClaimedNotification>, ReconciliationPortError> {
            Ok(self.claimed.lock().expect("not poisoned").pop_front())
        }

        async fn complete(
            &self,
            _claim: &ClaimedNotification,
            completion: NotificationCompletion,
        ) -> Result<(), ReconciliationPortError> {
            self.completions
                .lock()
                .expect("not poisoned")
                .push(completion);
            Ok(())
        }
    }

    struct FakeIntents {
        values: Vec<PublicationIntent>,
    }

    #[async_trait]
    impl PublicationIntents for FakeIntents {
        async fn for_namespace(
            &self,
            namespace: &RegistryNamespace,
        ) -> Result<Vec<PublicationIntent>, ReconciliationPortError> {
            Ok(self
                .values
                .iter()
                .filter(|intent| intent.claim().namespace() == namespace)
                .cloned()
                .collect())
        }

        async fn all(&self) -> Result<Vec<PublicationIntent>, ReconciliationPortError> {
            Ok(self.values.clone())
        }
    }

    struct FakeZot {
        value: ZotInspection,
    }

    struct FakeExecutor {
        fail: bool,
        applied: Arc<Mutex<Vec<ReconciliationAction>>>,
    }

    #[async_trait]
    impl ReconciliationActionExecutor for FakeExecutor {
        async fn apply(
            &self,
            action: &ReconciliationAction,
        ) -> Result<(), ReconciliationPortError> {
            if self.fail {
                return Err(ReconciliationPortError);
            }
            self.applied
                .lock()
                .expect("not poisoned")
                .push(action.clone());
            Ok(())
        }
    }

    #[async_trait]
    impl ZotRegistry for FakeZot {
        async fn inspect(
            &self,
            _reference: &ImmutableManifestReference,
        ) -> Result<ZotInspection, ReconciliationPortError> {
            Ok(self.value.clone())
        }
    }

    fn digest(byte: char) -> Sha256Digest {
        Sha256Digest::parse(format!("sha256:{}", byte.to_string().repeat(64))).expect("digest")
    }

    fn descriptor(byte: char, media_type: &str) -> OciDescriptor {
        OciDescriptor::new(
            digest(byte),
            100,
            OciMediaType::parse(media_type.to_owned()).expect("media type"),
        )
        .expect("descriptor")
    }

    fn intent() -> PublicationIntent {
        let owner = RegistryOwner::PlatformImage {
            image_key: PlatformImageKey::parse("rust-ubuntu").expect("key"),
        };
        let claim = NamespaceClaim::new(owner);
        let reference = ImmutableManifestReference::new(
            RegistryAuthority::parse("registry.example.test").expect("authority"),
            claim.namespace().clone(),
            digest('a'),
        );
        PublicationIntent::new(
            PublicationIntentId::from_uuid(Uuid::from_u128(1)),
            claim,
            reference,
            descriptor('a', "application/vnd.oci.image.index.v1+json"),
            registry_domain::PolicyVersion::parse("v1").expect("policy"),
            SupplyChainPolicy::without_signature(),
        )
        .expect("intent")
    }

    fn inspection() -> ZotInspection {
        let subject = digest('a');
        let manifest = descriptor('a', "application/vnd.oci.image.index.v1+json");
        let platform = PlatformDescriptor::new(
            descriptor('b', "application/vnd.oci.image.manifest.v1+json"),
            "linux",
            "amd64",
            None,
        )
        .expect("platform");
        let referrer = |kind, byte, artifact_type| {
            SupplyChainReferrer::new(
                kind,
                subject.clone(),
                descriptor(byte, artifact_type),
                OciMediaType::parse(artifact_type.to_owned()).expect("artifact type"),
            )
        };
        ZotInspection::Present {
            manifest,
            platforms: vec![platform],
            evidence: SupplyChainEvidence::new(
                subject.clone(),
                vec![
                    referrer(SupplyChainReferrerKind::Sbom, 'c', "application/spdx+json"),
                    referrer(
                        SupplyChainReferrerKind::Provenance,
                        'd',
                        "application/vnd.in-toto+json",
                    ),
                    referrer(
                        SupplyChainReferrerKind::Scan,
                        'e',
                        "application/vnd.hephaestus.vulnerability-scan.v1+json",
                    ),
                ],
            )
            .expect("evidence"),
        }
    }

    fn descriptor_mismatch_inspection() -> ZotInspection {
        let ZotInspection::Present {
            manifest: _,
            platforms,
            evidence,
        } = inspection()
        else {
            unreachable!();
        };
        ZotInspection::Present {
            manifest: OciDescriptor::new(
                digest('a'),
                101,
                OciMediaType::parse("application/vnd.oci.image.index.v1+json").expect("media type"),
            )
            .expect("descriptor"),
            platforms,
            evidence,
        }
    }

    fn verified_intent() -> PublicationIntent {
        let value = intent();
        let ZotInspection::Present {
            manifest,
            platforms,
            evidence,
        } = inspection()
        else {
            unreachable!();
        };
        let verification =
            VerifiedPublication::new(value.reference(), manifest, platforms, evidence)
                .expect("verification");
        value.record_verified(verification).expect("verified")
    }

    fn claim(
        namespace: Option<RegistryNamespace>,
        target: Option<ObservedTarget>,
    ) -> ClaimedNotification {
        ClaimedNotification {
            id: Uuid::from_u128(2),
            lease_token: Uuid::from_u128(3),
            repository_path: namespace.as_ref().map_or_else(
                || "unknown/repository".to_owned(),
                |value| value.as_str().to_owned(),
            ),
            namespace,
            target,
        }
    }

    fn reconciler(
        values: Vec<PublicationIntent>,
        zot: ZotInspection,
    ) -> RegistryReconciler<FakeInbox, FakeIntents, FakeZot> {
        RegistryReconciler::new(
            FakeInbox::default(),
            FakeIntents { values },
            FakeZot { value: zot },
        )
    }

    #[tokio::test]
    async fn duplicate_observations_propose_the_same_idempotent_verification() {
        let value = intent();
        let reducer = reconciler(vec![value.clone()], inspection());
        let target = ObservedTarget {
            digest: digest('a'),
            media_type: OciMediaType::parse("application/vnd.oci.image.index.v1+json")
                .expect("media type"),
        };
        let first = reducer
            .reduce_claimed(&claim(
                Some(value.claim().namespace().clone()),
                Some(target.clone()),
            ))
            .await
            .expect("first");
        let second = reducer
            .reduce_claimed(&claim(
                Some(value.claim().namespace().clone()),
                Some(target),
            ))
            .await
            .expect("second");
        assert_eq!(first, second);
        assert!(matches!(
            first.intents[0].actions.as_slice(),
            [ReconciliationAction::RecordVerified { .. }]
        ));
    }

    #[tokio::test]
    async fn process_next_completes_the_claim_but_never_executes_its_action() {
        let value = intent();
        let inbox = FakeInbox::default();
        inbox
            .claimed
            .lock()
            .expect("not poisoned")
            .push_back(claim(Some(value.claim().namespace().clone()), None));
        let reducer = RegistryReconciler::new(
            inbox.clone(),
            FakeIntents {
                values: vec![value],
            },
            FakeZot {
                value: inspection(),
            },
        );
        let result = reducer
            .process_next(Duration::from_secs(30))
            .await
            .expect("processed")
            .expect("claim");
        assert!(matches!(
            result.intents[0].actions.as_slice(),
            [ReconciliationAction::RecordVerified { .. }]
        ));
        assert_eq!(
            inbox.completions.lock().expect("not poisoned").as_slice(),
            [NotificationCompletion::Processed]
        );
    }

    #[tokio::test]
    async fn production_processing_leaves_the_claim_retryable_when_action_application_fails() {
        let value = intent();
        let inbox = FakeInbox::default();
        inbox
            .claimed
            .lock()
            .expect("not poisoned")
            .push_back(claim(Some(value.claim().namespace().clone()), None));
        let reducer = RegistryReconciler::new(
            inbox.clone(),
            FakeIntents {
                values: vec![value],
            },
            FakeZot {
                value: inspection(),
            },
        );
        let executor = FakeExecutor {
            fail: true,
            applied: Arc::default(),
        };
        assert!(
            reducer
                .process_next_and_apply(Duration::from_secs(30), &executor)
                .await
                .is_err()
        );
        assert!(inbox.completions.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn reordered_observation_never_overrides_the_authoritative_digest() {
        let value = verified_intent().approve().expect("approved");
        let reducer = reconciler(vec![value.clone()], inspection());
        let old_target = ObservedTarget {
            digest: digest('f'),
            media_type: OciMediaType::parse("application/vnd.oci.image.index.v1+json")
                .expect("media type"),
        };
        let reduction = reducer
            .reduce_claimed(&claim(
                Some(value.claim().namespace().clone()),
                Some(old_target),
            ))
            .await
            .expect("reduced");
        assert!(matches!(
            reduction.actions.as_slice(),
            [ReconciliationAction::ObservedDifferentTarget { .. }]
        ));
        assert!(reduction.intents[0].actions.is_empty());
    }

    #[tokio::test]
    async fn missed_event_is_found_by_full_authoritative_reconciliation() {
        let value = verified_intent().approve().expect("approved");
        let reducer = reconciler(vec![value], ZotInspection::Missing);
        let result = reducer.reconcile_all().await.expect("reconciled");
        assert!(matches!(
            result.intents[0].actions.as_slice(),
            [ReconciliationAction::MarkMissing { .. }]
        ));
    }

    #[tokio::test]
    async fn exact_graph_recovers_missing_content_without_a_new_approval() {
        let value = verified_intent()
            .approve()
            .expect("approved")
            .mark_missing()
            .expect("missing");
        let reducer = reconciler(vec![value], inspection());
        let result = reducer.reconcile_all().await.expect("reconciled");
        assert!(matches!(
            result.intents[0].actions.as_slice(),
            [ReconciliationAction::RestoreVerified { .. }]
        ));
    }

    #[tokio::test]
    async fn missing_approved_content_fails_closed() {
        let value = verified_intent().approve().expect("approved");
        let reducer = reconciler(vec![value.clone()], ZotInspection::Missing);
        let result = reducer
            .reduce_claimed(&claim(Some(value.claim().namespace().clone()), None))
            .await
            .expect("reduced");
        assert!(matches!(
            result.intents[0].actions.as_slice(),
            [ReconciliationAction::MarkMissing { .. }]
        ));
    }

    #[tokio::test]
    async fn descriptor_inconsistency_marks_approved_content_missing() {
        let value = verified_intent().approve().expect("approved");
        let reducer = reconciler(vec![value], descriptor_mismatch_inspection());
        let result = reducer.reconcile_all().await.expect("reconciled");
        assert!(matches!(
            result.intents[0].actions.as_slice(),
            [ReconciliationAction::MarkMissing {
                reason: Inconsistency::ManifestDescriptorMismatch,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn unknown_namespace_is_rejected_without_reading_zot() {
        let reducer = reconciler(Vec::new(), inspection());
        let result = reducer
            .reduce_claimed(&claim(None, None))
            .await
            .expect("reduced");
        assert_eq!(
            result.completion,
            NotificationCompletion::Rejected {
                failure_code: "unknown_namespace".to_owned()
            }
        );
        assert!(matches!(
            result.actions.as_slice(),
            [ReconciliationAction::OrphanNamespace { .. }]
        ));
    }
}
