//! Genuine `PostgreSQL` App Invocation composition; no physical VM execution proof.
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
#[allow(dead_code)]
#[path = "../../heph-std/run/postgres/tests/invocation_runtime/schema.rs"]
mod schema;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/authorization/runtime-authority-postgres/tests/invocation_pre_acquisition/schema.rs"]
mod authority_schema;

#[path = "qualified_invocation/bootstrap.rs"]
mod bootstrap;
mod qualified_invocation {
    mod acknowledgement;
    mod authority;
    pub mod bindings;
    mod composition;
    mod denials;
    mod fixture;
    mod launcher;
}
