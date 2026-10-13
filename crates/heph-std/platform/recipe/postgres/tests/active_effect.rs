//! Actual private113 authoritative active-claim reads; no provider IO.

#[path = "active_effect/authorization.rs"]
mod authorization;
#[path = "active_effect/support.rs"]
mod claim_fixture;
#[path = "support/fixtures.rs"]
#[allow(dead_code)] // Reuse verified actor pools and recipe declarations.
mod fixtures;
#[path = "execution_preparation/harness.rs"]
#[allow(dead_code)] // Shared preparation fixture; this target reads rather than imports.
mod harness;
#[path = "active_effect/integrity.rs"]
mod integrity;
#[path = "active_effect/reads.rs"]
mod reads;
#[path = "execution_preparation/schema.rs"]
mod schema;
#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
#[allow(dead_code)] // Existing exact authorization graph used by source fixtures.
mod seed;
#[path = "planning/fixture.rs"]
#[allow(dead_code)] // Published source fixture; unrelated planning helpers unused.
mod source_fixture;
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Existing verified identity and immutable fixture helpers.
mod support;

#[path = "active_effect/closure.rs"]
mod closure;
