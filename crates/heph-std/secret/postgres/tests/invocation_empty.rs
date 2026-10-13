//! Genuine B1 producer plus read-only saved-empty `SecretPG` verification.
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../run/postgres/tests/invocation_consumption/schema.rs"]
mod base_schema;
// Reuse the genuine catalogue bootstrap; Git planning cases remain in B1.
#[allow(dead_code)]
#[path = "../../../authorization/runtime-authority-postgres/tests/invocation_authority/bootstrap.rs"]
mod bootstrap;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../platform/recipe/postgres/tests/support/fixtures.rs"]
mod fixtures;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../forge/release/postgres/tests/instance_activation/schema.rs"]
mod preparation_schema;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../forge/release/postgres/tests/instance_activation/prepared_import.rs"]
mod prepared_import;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod prior_schema;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../platform/recipe/postgres/tests/planning/fixture.rs"]
mod source_fixture;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../authorization/authz-postgres/tests/postgres/support.rs"]
mod support;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../run/postgres/tests/invocation_runtime/fixture.rs"]
mod fixture;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../run/postgres/tests/invocation_runtime/owned_births.rs"]
mod owned_births;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../run/postgres/tests/invocation_runtime/graph.rs"]
mod graph;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
// Only the prior custody-completion helper is reused; its case target is separate.
#[allow(dead_code)]
#[path = "../../../run/postgres/tests/invocation_runtime/completion.rs"]
mod completion;
#[allow(dead_code)]
#[path = "../../../run/postgres/tests/invocation_runtime/schema.rs"]
mod schema;

// Reuse the genuine B1 harness only; no previous SQLx cases are registered.
#[allow(dead_code)]
#[path = "../../../authorization/runtime-authority-postgres/tests/invocation_authority/harness.rs"]
mod authority_harness;
mod invocation_empty {
    mod contradictions;
    mod fixture;
    mod integrity;
    mod phases;
    mod ports;
    mod positive;
    mod rights;
    mod secret_rows;
}
