//! Genuine116 retention/reopening followed by119 leases, with no physical VM claim.
#[path = "invocation_retained_reuse/acceptance.rs"]
mod acceptance;
#[path = "owned_first_ready/fixture.rs"]
#[allow(dead_code)] // Reuse actual Local birth and held106 completion.
mod birth;
#[path = "invocation_retained_reuse/execution.rs"]
mod execution;
#[path = "../../heph-std/platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Exact parsed recipe admissions and actor/worker pools.
mod fixtures;
#[path = "recipe_instance_import/fixture.rs"]
#[allow(dead_code)] // Genuine release/preparation fixture; unrelated importer cases unused.
mod instance_fixture;
#[path = "invocation_retained_reuse/preparation.rs"]
mod preparation;
#[path = "owned_ready_retention/fixture.rs"]
#[allow(dead_code)] // Reuse actual116 owning coordinator; no simulated closure receipt.
mod retained_fixture;
#[path = "../../heph-std/platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod schema;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../heph-std/platform/recipe/postgres/tests/planning/fixture.rs"]
#[allow(dead_code)] // Exact configured platform and frozen source catalog.
mod source_fixture;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Actual fixture middleware identities, never historic UUID reconstruction.
mod support;
