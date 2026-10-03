use capability_domain::CapabilitySlotKey;
use recipe_domain::ReleasePin;
use release_domain::ReleaseCommandKey;
use uuid::Uuid;

use crate::{CommandIdentity, DeploymentId};

pub const fn pin_key(pin: ReleasePin) -> (Uuid, Uuid) {
    (pin.release_id.as_uuid(), pin.release_agent_id.as_uuid())
}

pub fn import_operation(deployment: DeploymentId, name: &CapabilitySlotKey) -> Uuid {
    tagged_uuid(
        deployment.as_uuid(),
        b"hephaestus-recipe-import-operation-v1\0",
        name,
    )
}

pub fn mount_grant(revision: Uuid, slot: &CapabilitySlotKey) -> Uuid {
    tagged_uuid(revision, b"hephaestus-recipe-mount-grant-v1\0", slot)
}

fn tagged_uuid(namespace: Uuid, tag: &[u8], name: &CapabilitySlotKey) -> Uuid {
    let mut bytes = tag.to_vec();
    bytes.extend_from_slice(name.as_str().as_bytes());
    Uuid::new_v5(&namespace, &bytes)
}

pub fn import_key(
    deployment: DeploymentId,
    command: CommandIdentity,
    name: &CapabilitySlotKey,
    operation: Uuid,
) -> ReleaseCommandKey {
    ReleaseCommandKey::derive(
        "hephaestus-recipe-import-command-v1",
        &[
            deployment.as_uuid().as_bytes(),
            command.id().as_uuid().as_bytes(),
            name.as_str().as_bytes(),
            operation.as_bytes(),
        ],
    )
}
