//! Genuine terminal persistence with controlled exit/cleanup ports; no physical VM or WAL proof.
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/run/postgres/tests/invocation_consumption/schema.rs"]
mod base_schema;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/authorization/runtime-authority-postgres/tests/invocation_authority/bootstrap.rs"]
mod authority_bootstrap;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/platform/recipe/postgres/tests/support/fixtures.rs"]
mod fixtures;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/forge/release/postgres/tests/instance_activation/schema.rs"]
mod preparation_schema;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/forge/release/postgres/tests/instance_activation/prepared_import.rs"]
mod prepared_import;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod prior_schema;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/platform/recipe/postgres/tests/planning/fixture.rs"]
mod source_fixture;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/support.rs"]
mod support;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/run/postgres/tests/invocation_runtime/fixture.rs"]
mod fixture;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/run/postgres/tests/invocation_runtime/owned_births.rs"]
mod owned_births;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/run/postgres/tests/invocation_runtime/graph.rs"]
mod graph;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/run/postgres/tests/invocation_runtime/completion.rs"]
mod completion;
// Helper-only reuse: original case modules are not registered in this target.
#[path = "terminal_success_private/schema.rs"]
mod authority_schema;
#[path = "qualified_invocation/bootstrap.rs"]
mod bootstrap;
#[allow(dead_code)]
#[path = "../../heph-std/run/postgres/tests/invocation_runtime/schema.rs"]
mod schema;
// Reuse helpers only; no earlier native case registrations.
#[allow(dead_code)]
mod qualified_invocation {
    #[path = "../qualified_invocation/bindings.rs"]
    pub mod bindings;
    #[path = "../qualified_invocation/composition.rs"]
    pub mod composition;
    #[path = "../qualified_invocation/fixture.rs"]
    pub mod fixture;
}
// Saved-empty effect ports panic if any secret IO is attempted.
#[allow(dead_code)]
#[path = "../../heph-std/secret/postgres/tests/invocation_empty/ports.rs"]
mod secret_ports;
mod terminal_success_private {
    #[path = "arguments.rs"]
    mod arguments;
    mod authenticated_stamp;
    mod cleanup;
    #[path = "composition.rs"]
    mod composition;
    mod fixture;
    mod observations;
    #[path = "owning_transaction.rs"]
    mod owning_transaction;
    mod running;
    #[path = "same_transaction.rs"]
    mod same_transaction;
    mod success;
}
