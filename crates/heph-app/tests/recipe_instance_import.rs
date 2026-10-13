//! Actual private113 recipe claim -> real107 import -> trusted106 completion.
//! Metadata/service proof only: no backing creation, VM or named dispatch.

#[path = "../../heph-std/platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Reuse parsed intent/admission and separately configured actor pool.
mod fixtures;
#[path = "../../heph-std/platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod schema;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../heph-std/platform/recipe/postgres/tests/planning/fixture.rs"]
#[allow(dead_code)] // Share exact published runtime contract/policy; no fake provider result.
mod source_fixture;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Verified fixture identities; unrelated authorization helpers are unused.
mod support;

#[path = "recipe_instance_import/authorization.rs"]
mod authorization;
#[path = "recipe_instance_import/closure.rs"]
mod closure;
#[path = "recipe_instance_import/collisions.rs"]
mod collisions;
#[path = "recipe_instance_import/converter.rs"]
mod converter;
#[path = "recipe_instance_import/failures.rs"]
mod failures;
#[path = "recipe_instance_import/hash_vectors.rs"]
mod hash_vectors;
#[path = "recipe_instance_import/fixture.rs"]
mod instance_fixture;
#[path = "recipe_instance_import/races.rs"]
mod races;
#[path = "recipe_instance_import/success.rs"]
mod success;

#[path = "recipe_instance_import/ordinary.rs"]
mod ordinary;
#[path = "recipe_instance_import/reservations.rs"]
mod reservations;

#[path = "recipe_instance_import/parameter_vectors.rs"]
mod parameter_vectors;
