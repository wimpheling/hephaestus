//! Structural regression tests for the durable product-event boundary.

const MIGRATION: &str = include_str!("../../../migrations/0010_durable_application_events.sql");
const GATEWAY_EVENTS_MIGRATION: &str =
    include_str!("../../../migrations/0044_gateway_product_events.sql");

#[path = "event_durability/atomicity.rs"]
mod atomicity;
#[path = "event_durability/gateway.rs"]
mod gateway;
#[path = "event_durability/schema_contract.rs"]
mod schema_contract;
#[path = "event_durability/support.rs"]
mod support;
#[path = "event_durability/visibility.rs"]
mod visibility;
