//! Actual first-birth Ready verification and protected completion custody.
//! Native filesystem/DB proof only; no VM, recipe engine or startup activation.

#[path = "owned_first_ready/admission.rs"]
mod admission;
#[path = "owned_first_ready/cancellation.rs"]
mod cancellation;
#[path = "owned_first_ready/fixture.rs"]
mod fixture;
#[path = "../../heph-std/platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Reuse genuine parsed/admitted intents and configured pools.
mod fixtures;
#[path = "recipe_instance_import/fixture.rs"]
#[allow(dead_code)] // Shared whole preparation Harness; no duplicate activation fixture.
mod instance_fixture;
#[path = "owned_first_ready/physical.rs"]
mod physical;
#[path = "../../heph-std/platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod schema;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../heph-std/platform/recipe/postgres/tests/planning/fixture.rs"]
#[allow(dead_code)] // Share frozen published contract/policy seeding.
mod source_fixture;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Existing verified identities; no synthetic creator identity.
mod support;
