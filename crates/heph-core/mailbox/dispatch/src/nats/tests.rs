use super::MailboxDispatchCommand;
use mailbox_domain::{MailboxEventId, MailboxId, MailboxOperationIdentity};

#[test]
fn command_serializes_only_stable_identifiers() {
    let mailbox_id = MailboxId::new();
    let event_id = MailboxEventId::new();
    let command = MailboxDispatchCommand {
        operation_id: MailboxOperationIdentity::dispatch(mailbox_id, event_id, 1).id(),
        event_id,
    };

    let encoded = serde_json::to_value(command).expect("serialize command");
    assert_eq!(encoded.as_object().expect("object").len(), 2);
    assert!(encoded.get("operation_id").is_some());
    assert!(encoded.get("mailbox_event_id").is_some());
}
