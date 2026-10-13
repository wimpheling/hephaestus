//! Actual original107 creation observation and readonly manager/worker separation.
//! No runtime VM drain or physical cleanup claim is made by zero-Run cases.
#[path = "prepared_instance_safety/authority.rs"]
mod authority;
#[path = "../../../../platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod base_schema;
#[path = "instance_activation/bootstrap.rs"]
mod bootstrap;
#[path = "prepared_instance_safety/creation.rs"]
mod creation;
#[path = "prepared_instance_safety/fixture.rs"]
mod fixture;
#[path = "../../../../platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Reuse immutable intent and real actor pool fixtures.
mod fixtures;
#[path = "prepared_instance_safety/integrity.rs"]
mod integrity;
#[path = "instance_activation/schema.rs"]
mod preparation_schema;
#[path = "instance_activation/prepared_import.rs"]
mod prepared_import;
#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../../../platform/recipe/postgres/tests/planning/fixture.rs"]
#[allow(dead_code)] // Reuse exact released workload and checked platform observation.
mod source_fixture;
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Genuine seeded users; unused FGA fixture projections are unrelated.
mod support;
