//! Exact inert Cooking fixture constants required by the reused build facade.

// Copied from examples/cooking/tests/scenario/config.rs.
pub const MODEL_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000003");
pub const RELAY_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000004");

// Copied from examples/cooking/tests/scenario/seed.rs.
pub const INBOUND_SENTINEL: &str = "cooking-inbound-only-fixture-sentinel";
pub const MODEL_SENTINEL: &str = "cooking-model-only-fixture-sentinel-724c";
pub const RELAY_SENTINEL: &str = "cooking-relay-only-fixture-sentinel-819e";

// Copied from examples/cooking/tests/scenario/credentials.rs.
pub const INBOUND_ROTATED_SENTINEL: &str = "cooking-inbound-rotated-fixture-sentinel-157a";
pub const MODEL_ROTATED_SENTINEL: &str = "cooking-model-rotated-fixture-sentinel-936f";
pub const RELAY_ROTATED_SENTINEL: &str = "cooking-relay-rotated-fixture-sentinel-482b";
