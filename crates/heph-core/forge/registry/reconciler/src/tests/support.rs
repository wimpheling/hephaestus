use crate::*;
use async_trait::async_trait;
use registry_domain::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType, PlatformDescriptor,
    PlatformImageKey, PublicationIntent, PublicationIntentId, RegistryAuthority, RegistryNamespace,
    RegistryOwner, Sha256Digest, SupplyChainEvidence, SupplyChainPolicy, SupplyChainReferrer,
    SupplyChainReferrerKind, VerifiedPublication,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone, Default)]
pub(super) struct FakeInbox {
    pub(super) claimed: Arc<Mutex<VecDeque<ClaimedNotification>>>,
    pub(super) completions: Arc<Mutex<Vec<NotificationCompletion>>>,
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

pub(super) struct FakeIntents {
    pub(super) values: Vec<PublicationIntent>,
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

pub(super) struct FakeZot {
    pub(super) value: ZotInspection,
}

pub(super) struct FakeExecutor {
    pub(super) fail: bool,
    pub(super) applied: Arc<Mutex<Vec<ReconciliationAction>>>,
}

#[async_trait]
impl ReconciliationActionExecutor for FakeExecutor {
    async fn apply(&self, action: &ReconciliationAction) -> Result<(), ReconciliationPortError> {
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

pub(super) fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::parse(format!("sha256:{}", byte.to_string().repeat(64))).expect("digest")
}

pub(super) fn descriptor(byte: char, media_type: &str) -> OciDescriptor {
    OciDescriptor::new(
        digest(byte),
        100,
        OciMediaType::parse(media_type.to_owned()).expect("media type"),
    )
    .expect("descriptor")
}

pub(super) fn intent() -> PublicationIntent {
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

pub(super) fn inspection() -> ZotInspection {
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

pub(super) fn descriptor_mismatch_inspection() -> ZotInspection {
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

pub(super) fn verified_intent() -> PublicationIntent {
    let value = intent();
    let ZotInspection::Present {
        manifest,
        platforms,
        evidence,
    } = inspection()
    else {
        unreachable!();
    };
    let verification = VerifiedPublication::new(value.reference(), manifest, platforms, evidence)
        .expect("verification");
    value.record_verified(verification).expect("verified")
}

pub(super) fn claim(
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

pub(super) fn reconciler(
    values: Vec<PublicationIntent>,
    zot: ZotInspection,
) -> RegistryReconciler<FakeInbox, FakeIntents, FakeZot> {
    RegistryReconciler::new(
        FakeInbox::default(),
        FakeIntents { values },
        FakeZot { value: zot },
    )
}
