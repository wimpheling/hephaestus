//! Actual private119 lease/progress SQL and Local births; no physical VM proof.

#[path = "invocation_consumption/schema.rs"]
mod base_schema;
#[path = "invocation_runtime/bootstrap.rs"]
mod bootstrap;
#[path = "../../../platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Reuse exact actor admission and explicit role pools.
mod fixtures;
#[path = "../../../forge/release/postgres/tests/instance_activation/schema.rs"]
mod preparation_schema;
#[path = "../../../forge/release/postgres/tests/instance_activation/prepared_import.rs"]
mod prepared_import;
#[path = "../../../platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod prior_schema;
#[path = "../../../authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../../platform/recipe/postgres/tests/planning/fixture.rs"]
#[allow(dead_code)] // Reuse exact published contracts/policies; unrelated planners unused.
mod source_fixture;
#[path = "../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Real fixture middleware identities and source/project grants.
mod support;

#[path = "invocation_runtime/acquisition.rs"]
mod acquisition;
#[path = "invocation_runtime/completion.rs"]
mod completion;
#[path = "invocation_runtime/denial.rs"]
mod denial;
#[path = "invocation_runtime/fixture.rs"]
mod fixture;
#[path = "invocation_runtime/graph.rs"]
mod graph;
#[path = "invocation_runtime/integrity.rs"]
mod integrity;
#[path = "invocation_runtime/locking.rs"]
mod locking;
#[path = "invocation_runtime/owned_births.rs"]
mod owned_births;
#[path = "invocation_runtime/schema.rs"]
mod schema;

#[path = "invocation_runtime/partial_acquisition.rs"]
mod partial_acquisition;
#[path = "invocation_runtime/reservations.rs"]
mod reservations;
