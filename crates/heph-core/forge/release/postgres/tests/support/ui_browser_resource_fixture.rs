// Shared real-PostgreSQL fixture for focused UI resource tests.

#[path = "ui_browser_resource_fixture/handoffs.rs"]
mod handoffs;
#[path = "ui_browser_resource_fixture/installations.rs"]
mod installations;
#[path = "ui_browser_resource_fixture/model.rs"]
mod model;
#[path = "ui_browser_resource_fixture/seed.rs"]
mod seed;
#[path = "ui_browser_resource_fixture/ui_graph.rs"]
mod ui_graph;

// These helpers are part of the fixture API used by the release-postgres
// integration target, even when another consumer includes only part of it.
#[allow(unused_imports)]
pub use handoffs::{
    insert_authenticated_child, insert_authenticated_child_for, insert_managed_authenticated_child,
};
// The alias preserves the original fixture API for support consumers that
// inspect the complete identity graph directly.
#[allow(dead_code)]
pub type Fixture = model::Fixture;
pub use model::fixture_session_secret;
pub use seed::seed_fixture_reusing_installation_helpers;
#[allow(unused_imports)]
pub use seed::seed_fixture_reusing_installation_helpers_draft;
