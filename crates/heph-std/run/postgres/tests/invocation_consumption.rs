//! Actual private117 worker metadata consumption; no runtime or VM proof.

#[path = "../../../forge/release/postgres/tests/instance_activation/fixture.rs"]
#[allow(dead_code)] // Shared genuine113/106/107/114 birth fixture; no synthetic Ready.
mod activation_fixture;
#[path = "invocation_consumption/schema.rs"]
mod base_schema;
#[path = "invocation_consumption/bootstrap.rs"]
mod bootstrap;
#[path = "../../../platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Reuse exact actor admission and explicit role pools.
mod fixtures;
#[path = "../../../forge/release/postgres/tests/instance_activation/owned_birth.rs"]
mod owned_birth;
#[path = "../../../forge/release/postgres/tests/instance_activation/schema.rs"]
mod preparation_schema;
#[path = "../../../forge/release/postgres/tests/instance_activation/prepared_import.rs"]
mod prepared_import;
#[path = "../../../platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod prior_schema;
#[path = "../../../forge/release/postgres/tests/instance_invocation/fixture.rs"]
#[allow(dead_code)] // Reuse actual115 admission; raw actor mutation helpers unused.
mod producer_fixture;
#[path = "../../../authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../../platform/recipe/postgres/tests/planning/fixture.rs"]
#[allow(dead_code)] // Reuse exact published contracts/policies; unrelated planners unused.
mod source_fixture;
#[path = "../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Real fixture middleware identities and source/project grants.
mod support;

#[path = "invocation_consumption/authority.rs"]
mod authority;
#[path = "invocation_consumption/consumption.rs"]
mod consumption;
#[path = "invocation_consumption/fixture.rs"]
mod fixture;
#[path = "invocation_consumption/integrity.rs"]
mod integrity;
#[path = "invocation_consumption/locking.rs"]
mod locking;
