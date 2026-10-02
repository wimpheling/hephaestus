use super::super::{JournalPurpose, codec::digest};
use runtime_types::VolumeId;
use tempfile::TempDir;
use uuid::Uuid;
use volume_trait::{
    OwnedVolumeRegistration, OwnedVolumeRegistrationReceipt, VolumeCreationIdentity,
    VolumeCreationIdentityWire, VolumeCreationOperationId, VolumeCreationSealId,
    VolumeOwnershipScopeId, VolumeRegistration, VolumeRootNamespaceId,
};

pub struct Fixture {
    pub root: TempDir,
    pub purpose: JournalPurpose,
    pub receipt: OwnedVolumeRegistrationReceipt,
    pub owner_namespace: VolumeRootNamespaceId,
}
impl Fixture {
    pub fn new() -> Self {
        let root = TempDir::new().expect("private fixture root");
        let identity = VolumeCreationIdentity::try_from(VolumeCreationIdentityWire {
            scope_id: VolumeOwnershipScopeId::from_uuid(Uuid::new_v4()).expect("scope"),
            operation_id: VolumeCreationOperationId::from_uuid(Uuid::new_v4()).expect("operation"),
            command_id: Uuid::new_v4(),
            effect_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            resource_version: 2,
            generation: 1,
            input_hash: [1; 32],
            intent_hash: [2; 32],
        })
        .expect("identity");
        let intent = OwnedVolumeRegistration::new(
            VolumeRegistration::new(
                VolumeId::new(),
                Uuid::new_v4(),
                16 * 1024 * 1024,
                Uuid::new_v4(),
            )
            .expect("registration"),
            identity,
        );
        let registration_hash = digest(&intent.canonical_bytes());
        let receipt = OwnedVolumeRegistrationReceipt {
            intent,
            seal_id: VolumeCreationSealId::from_uuid(Uuid::new_v4()).expect("seal"),
            registration_hash,
            first_request_id: Uuid::new_v4(),
            event_id: Uuid::new_v4(),
            event_cursor: 3,
            event_aggregate_version: 2,
        };
        let owner_namespace =
            VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).expect("root namespace");
        let purpose =
            JournalPurpose::new(&receipt, "host-fixture", root.path(), owner_namespace, 1)
                .expect("purpose");
        Self {
            root,
            purpose,
            receipt,
            owner_namespace,
        }
    }
    pub fn namespace(&self) -> std::path::PathBuf {
        self.root.path().join(self.purpose.namespace_name())
    }
    pub fn canonical(&self) -> std::path::PathBuf {
        self.root.path().join(self.purpose.canonical_name())
    }
}

/// An unrelated parallel subprocess may briefly inherit CLOEXEC descriptors
/// between fork and exec. Model restart retry without weakening production flock.
pub fn restart_lock(purpose: &JournalPurpose) -> super::super::OwnedJournalLock {
    for _ in 0..100 {
        match super::super::OwnedJournalLock::acquire(purpose.clone()) {
            Ok(lock) => return lock,
            Err(super::super::JournalError::Locked) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("restart lock failed without transient contention: {error}"),
        }
    }
    panic!("restart lock remained held after a bounded retry window");
}
