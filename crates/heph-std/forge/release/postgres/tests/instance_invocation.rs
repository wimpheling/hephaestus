//! Actual private115 Invocation admission; execution remains unsupported.

#[path = "../../../../platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod base_schema;
#[path = "../../../../platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Share checked declaration/admission and explicit actor role pool.
mod fixtures;
#[path = "instance_activation/schema.rs"]
mod preparation_schema;
#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../../../platform/recipe/postgres/tests/planning/fixture.rs"]
#[allow(dead_code)] // Share exact released policy and published source fixtures.
mod source_fixture;
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Real middleware fixture identities; unrelated FGA fixtures unused.
mod support;

#[path = "instance_activation/fixture.rs"]
#[allow(dead_code)]
// Shared114 physical/prior-state fixtures are not all used by this slotless target.
mod activation_fixture;
#[path = "instance_activation/bootstrap.rs"]
mod bootstrap;
#[path = "instance_activation/owned_birth.rs"]
#[allow(dead_code)]
// Shared114 physical/prior-state fixtures are not all used by this slotless target.
mod owned_birth;
#[path = "instance_activation/prepared_import.rs"]
mod prepared_import;

#[path = "instance_invocation/admission.rs"]
mod admission;
#[path = "instance_invocation/authority.rs"]
mod authority;
#[path = "instance_invocation/fixture.rs"]
mod fixture;
#[path = "instance_invocation/integrity.rs"]
mod integrity;
#[path = "instance_invocation/locking.rs"]
mod locking;
