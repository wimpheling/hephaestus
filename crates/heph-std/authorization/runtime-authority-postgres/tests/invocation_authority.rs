//! Genuine119 producer/fullset/session metadata; no physical VM proof.
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../run/postgres/tests/invocation_consumption/schema.rs"]
mod base_schema;
// Fixture-only catalogue adaptation declares requirements before publication.
#[path = "invocation_authority/bootstrap.rs"]
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
#[path = "../../authz-postgres/tests/postgres/seed.rs"]
mod seed;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../../platform/recipe/postgres/tests/planning/fixture.rs"]
mod source_fixture;
// Reuse only genuine native prerequisite helpers; other entry points are
// exercised by their own frozen targets and are not duplicated here.
#[allow(dead_code)]
#[path = "../../authz-postgres/tests/postgres/support.rs"]
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
mod invocation_authority {
    mod git;
    mod harness;
    mod integrity;
    mod issuance;
    mod locking;
    mod mounts;
    mod phases;
    mod vectors;
    mod withdrawal;
}
