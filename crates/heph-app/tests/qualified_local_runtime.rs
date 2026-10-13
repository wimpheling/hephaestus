//! Helper-only qualified Local publication composition; no native cases or drain proof.

// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/platform/recipe/postgres/tests/support/fixtures.rs"]
mod fixtures;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/support.rs"]
mod support;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod prior_schema;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/run/postgres/tests/invocation_consumption/schema.rs"]
mod base_schema;
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
#[path = "../../heph-std/run/postgres/tests/invocation_runtime/schema.rs"]
mod schema;
// Helper-only reuse: original case modules are not registered in this target.
#[allow(dead_code)]
#[path = "../../heph-std/authorization/runtime-authority-postgres/tests/invocation_pre_acquisition/schema.rs"]
mod authority_schema;

mod qualified_local_runtime {
    // Compile-only helper registration; native cases are not registered yet.
    #[allow(dead_code)]
    #[path = "../support/rpc/qualified_local_publication.rs"]
    mod build_helpers;
    // Compile-only helper registration; native cases are not registered yet.
    #[allow(dead_code)]
    mod publication;
    // Compile-only helper registration; native cases are not registered yet.
    #[allow(dead_code)]
    mod owned_births;
    // Compile-only helper registration; native cases are not registered yet.
    #[allow(dead_code)]
    mod source_variants;
    // Compile-only helper registration; native cases are not registered yet.
    #[allow(dead_code)]
    mod bootstrap;
    // Compile-only helper registration; native cases are not registered yet.
    #[allow(dead_code)]
    mod publication_provider;
    // Compile-only helper registration; native cases are not registered yet.
    #[allow(dead_code)]
    mod publication_backend;
    // Compile-only helper registration; native cases are not registered yet.
    #[allow(dead_code)]
    mod publication_setup;
    // Inert constants required by inherited build helpers; no scenario registration.
    #[allow(dead_code)]
    #[path = "cooking_constants.rs"]
    mod cooking;
}
