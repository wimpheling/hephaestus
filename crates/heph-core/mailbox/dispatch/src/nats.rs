//! Durable mailbox command transport and VM-completion integration.
//!
//! `PostgreSQL` remains authoritative for every delivery transition. This crate
//! deliberately carries only mailbox-event and stable-operation identifiers in
//! `JetStream`; it never serializes event bodies, capability bearers, or mutable
//! attempt state.

#[path = "nats/contract.rs"]
mod contract;
#[path = "nats/handler.rs"]
mod handler;
#[path = "nats/observers.rs"]
mod observers;
#[path = "nats/publisher.rs"]
mod publisher;
#[path = "nats/topology.rs"]
mod topology;

#[cfg(test)]
#[path = "nats/tests.rs"]
mod tests;

/// Wake-up command subject for delivery eligibility changes.
pub const MAILBOX_WAKE_SUBJECT: &str = "heph.mailbox.v1.wake";
/// Dispatch command subject for one already-eligible mailbox event.
pub const MAILBOX_DISPATCH_SUBJECT: &str = "heph.mailbox.v1.dispatch";
/// Retry command subject for an event whose `PostgreSQL` backoff has elapsed.
pub const MAILBOX_RETRY_SUBJECT: &str = "heph.mailbox.v1.retry";
/// Cancellation command subject for an accepted mailbox event.
pub const MAILBOX_CANCEL_SUBJECT: &str = "heph.mailbox.v1.cancel";
/// Recovery command subject for a stale mailbox claim or incomplete attempt.
pub const MAILBOX_RECOVERY_SUBJECT: &str = "heph.mailbox.v1.recover";

/// All versioned mailbox command subjects owned by this transport.
pub const MAILBOX_COMMAND_SUBJECTS: [&str; 5] = [
    MAILBOX_WAKE_SUBJECT,
    MAILBOX_DISPATCH_SUBJECT,
    MAILBOX_RETRY_SUBJECT,
    MAILBOX_CANCEL_SUBJECT,
    MAILBOX_RECOVERY_SUBJECT,
];

const MAILBOX_COMMAND_STREAM: &str = "HEPH_MAILBOX_COMMANDS";
const MAILBOX_COMMAND_CONSUMER: &str = "mailbox-dispatcher-v1";

pub use contract::{
    MailboxDispatchCommand, MailboxDispatchStore, MailboxDispatchStoreError, MailboxOutboxRecord,
};
pub use handler::{MailboxCommandError, MailboxCommandHandler, NatsMailboxCommandHandler};
pub use observers::{MailboxRunCompletion, MailboxRunResources};
pub use publisher::{MailboxOutboxPublishError, MailboxOutboxPublisher};
pub use topology::{MailboxTopologyError, ensure_mailbox_jetstream_topology};
