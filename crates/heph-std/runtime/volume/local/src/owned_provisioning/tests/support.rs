use super::{
    legacy::Legacy,
    metadata::{Metadata, State},
};
use crate::{
    LocalVolumeConfig, LocalVolumeStore, OwnedVolumeMetadata, VolumeRootOwner,
    owned_journal::JournalPurpose,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use runtime_types::VolumeId;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;
use uuid::Uuid;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingPurpose, OwnedProvisioningClaim, OwnedProvisioningContext,
    OwnedVolumeRegistration, OwnedVolumeRegistrationReceipt, VolumeCreationIdentity,
    VolumeCreationIdentityWire, VolumeCreationOperationId, VolumeCreationSealId,
    VolumeOwnershipScopeId, VolumeRegistration,
};

pub struct Fixture {
    pub root: TempDir,
    pub tools: TempDir,
    pub store: LocalVolumeStore,
    pub identity: AuthenticatedIdentity,
    pub request: BeginOwnedProvisioning,
    pub state: Arc<Mutex<State>>,
    pub owner: Arc<VolumeRootOwner>,
    pub real_mkfs: &'static str,
}
impl Fixture {
    pub fn new() -> Self {
        let root = TempDir::new().expect("private root");
        let tools = TempDir::new().expect("tools");
        let owner =
            Arc::new(VolumeRootOwner::initialize(root.path(), "host-fixture").expect("root owner"));
        let identity = AuthenticatedIdentity::new(
            UserId::new(),
            "fixture-issuer",
            "fixture-subject",
            "null".parse().expect("fixture null claims"),
            RequestId::new(),
        );
        let receipt = receipt(identity.user_id.as_uuid());
        let purpose = OwnedBackingPurpose::new(
            receipt,
            "host-fixture".into(),
            root.path().into(),
            owner.namespace_id(),
            1,
        )
        .expect("purpose");
        let request = BeginOwnedProvisioning {
            purpose: purpose.clone(),
            operation_id: Uuid::new_v4(),
            expected_generation: 0,
        };
        let context = OwnedProvisioningContext {
            claim: OwnedProvisioningClaim {
                purpose,
                operation_id: request.operation_id,
                generation: 1,
                request_id: identity.request_id.as_uuid(),
                event_id: Uuid::new_v4(),
            },
            observation: None,
        };
        let state = Arc::new(Mutex::new(State {
            context,
            authorized: true,
            begins: 0,
            reject_begin: None,
            fail_ready_once: false,
            revoke_on_format_record: false,
        }));
        let real_mkfs = ["/usr/sbin/mkfs.ext4", "/sbin/mkfs.ext4"]
            .into_iter()
            .find(|path| std::path::Path::new(path).is_file())
            .expect("install e2fsprogs for native owned provisioning tests");
        let real_fsck = std::path::Path::new(real_mkfs)
            .parent()
            .expect("parent")
            .join("e2fsck");
        assert!(real_fsck.is_file(), "install readonly e2fsck");
        symlink(real_fsck, tools.path().join("e2fsck")).expect("fsck");
        let config = LocalVolumeConfig {
            volume_root: root.path().into(),
            transient_runtime_roots: Vec::new(),
            host_id: "host-fixture".into(),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: tools.path().join("mkfs.ext4"),
        };
        let store = LocalVolumeStore::new(Arc::new(Legacy), config)
            .expect("store")
            .with_owned_metadata(crate::OwnedVolumeMetadata::new(
                Arc::new(Metadata {
                    state: state.clone(),
                    worker: false,
                }),
                Arc::new(Metadata {
                    state: state.clone(),
                    worker: true,
                }),
                owner.clone(),
            ))
            .expect("owned composition");
        let result = Self {
            root,
            tools,
            store,
            identity,
            request,
            state,
            owner,
            real_mkfs,
        };
        result.formatter(&format!("exec '{real_mkfs}' \"$@\"\n"));
        result
    }
    pub fn formatter(&self, body: &str) {
        let path = self.tools.path().join("mkfs.ext4");
        let script = format!(
            "#!/bin/sh\nprintf 'format\\n' >> '{}'\n{body}",
            self.tools.path().join("formats").display()
        );
        fs::write(&path, script).expect("formatter");
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .expect("formatter permissions");
    }
    pub fn format_count(&self) -> usize {
        fs::read_to_string(self.tools.path().join("formats"))
            .unwrap_or_default()
            .lines()
            .count()
    }
    pub fn canonical(&self) -> std::path::PathBuf {
        self.request.purpose.canonical_path()
    }
    pub fn journal_purpose(&self) -> JournalPurpose {
        let purpose = &self.request.purpose;
        JournalPurpose::new(
            purpose.receipt(),
            purpose.host(),
            purpose.root(),
            purpose.owner_namespace(),
            purpose.birth_generation(),
        )
        .expect("journal purpose")
    }
    pub fn claim(&self) -> OwnedProvisioningClaim {
        self.state.lock().expect("state").context.claim.clone()
    }
    pub fn actor_metadata(&self) -> Arc<Metadata> {
        Arc::new(Metadata {
            state: self.state.clone(),
            worker: false,
        })
    }
    pub fn uncomposed(&self) -> LocalVolumeStore {
        LocalVolumeStore::new(
            Arc::new(Legacy),
            LocalVolumeConfig {
                volume_root: self.root.path().into(),
                transient_runtime_roots: Vec::new(),
                host_id: "host-fixture".into(),
                lease_duration: Duration::from_secs(30),
                mkfs_ext4: self.tools.path().join("mkfs.ext4"),
            },
        )
        .expect("uncomposed")
    }
    pub fn wrong_worker_store(&self) -> LocalVolumeStore {
        self.uncomposed()
            .with_owned_metadata(OwnedVolumeMetadata::new(
                self.actor_metadata(),
                self.actor_metadata(),
                self.owner.clone(),
            ))
            .expect("wrong role composition")
    }
}
fn receipt(actor_id: Uuid) -> OwnedVolumeRegistrationReceipt {
    let creation = VolumeCreationIdentity::try_from(VolumeCreationIdentityWire {
        scope_id: VolumeOwnershipScopeId::from_uuid(Uuid::new_v4()).expect("scope"),
        operation_id: VolumeCreationOperationId::from_uuid(Uuid::new_v4()).expect("operation"),
        command_id: Uuid::new_v4(),
        effect_id: Uuid::new_v4(),
        attempt_id: Uuid::new_v4(),
        actor_id,
        request_id: Uuid::new_v4(),
        resource_version: 2,
        generation: 1,
        input_hash: [1; 32],
        intent_hash: [2; 32],
    })
    .expect("creation");
    let intent = OwnedVolumeRegistration::new(
        VolumeRegistration::new(
            VolumeId::new(),
            Uuid::new_v4(),
            16 * 1024 * 1024,
            Uuid::new_v4(),
        )
        .expect("registration"),
        creation,
    );
    let registration_hash = Sha256::digest(intent.canonical_bytes()).into();
    OwnedVolumeRegistrationReceipt {
        intent,
        seal_id: VolumeCreationSealId::from_uuid(Uuid::new_v4()).expect("seal"),
        registration_hash,
        first_request_id: Uuid::new_v4(),
        event_id: Uuid::new_v4(),
        event_cursor: 1,
        event_aggregate_version: 1,
    }
}
