//! Descriptor-level policy checks for the shared application protocol.

#[path = "descriptor_policy/common.rs"]
mod common;
#[path = "descriptor_policy/events.rs"]
mod events;
#[path = "descriptor_policy/inventory.rs"]
mod inventory;
#[path = "descriptor_policy/payload.rs"]
mod payload;
#[path = "descriptor_policy/schema.rs"]
mod schema;
#[path = "descriptor_policy/sensitive.rs"]
mod sensitive;
#[path = "descriptor_policy/watch.rs"]
mod watch;
