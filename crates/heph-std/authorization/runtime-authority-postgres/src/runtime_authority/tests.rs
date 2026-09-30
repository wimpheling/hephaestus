use super::gateway_helpers::{GatewaySnapshotBindingRow, gateway_mailbox_binding};
use super::gateway_helpers::{operations, resource_kind};
use capability_domain::{CapabilityOperation, CapabilityResourceKind};
use uuid::Uuid;

#[test]
fn gateway_mailbox_snapshot_binding_is_exact_publish_authority() {
    let binding_id = Uuid::new_v4();
    let mailbox_id = Uuid::new_v4();
    let binding = gateway_mailbox_binding(GatewaySnapshotBindingRow {
        binding_id,
        grant_id: Uuid::new_v4(),
        slot_key: "accepted-event".to_owned(),
        resource_id: mailbox_id,
    })
    .expect("valid exact mailbox publication binding");
    assert_eq!(binding.id().as_uuid(), binding_id);
    assert_eq!(binding.resource().kind, CapabilityResourceKind::Mailbox);
    assert_eq!(binding.resource().id, mailbox_id);
    assert_eq!(
        binding.granted_operations().collect::<Vec<_>>(),
        vec![CapabilityOperation::Publish]
    );
}

#[test]
fn parses_mailbox_publish_persisted_authority() {
    assert_eq!(
        resource_kind("mailbox").expect("mailbox is a supported persisted resource"),
        CapabilityResourceKind::Mailbox
    );
    assert_eq!(
        operations(&["publish".to_owned()]).expect("publish is supported"),
        vec![CapabilityOperation::Publish]
    );
}
