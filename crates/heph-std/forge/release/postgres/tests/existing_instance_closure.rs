//! Read-only original closure under a genuine fresh manager continuation.
//! Zero-Run observations are database safety evidence, not physical VM cleanup.
#[path = "../../../../platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod base_schema;
#[path = "instance_activation/bootstrap.rs"]
mod bootstrap;
#[path = "prepared_instance_closure/fixture.rs"]
mod closure_fixture;
#[path = "../../../../platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Reuse immutable intent and real actor pool fixtures.
mod fixtures;
#[path = "prepared_instance_safety/fixture.rs"]
#[allow(dead_code)]
// Reuse genuine birth only; its close/changed helpers belong to old safety tests.
mod original_fixture;
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

#[path = "existing_instance_closure/behavior.rs"]
mod behavior;
#[path = "existing_instance_closure/fixture.rs"]
mod fixture;
#[path = "prepared_remove_resume/fixture.rs"]
#[allow(dead_code)]
// Reuse actual admission/birth; its preparation helper belongs to takeover tests.
mod resume_fixture;
