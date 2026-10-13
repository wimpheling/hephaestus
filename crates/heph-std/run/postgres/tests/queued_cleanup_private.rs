//! Genuine queued private cleanup lineage and controlled owning callbacks over `PostgreSQL`.

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

#[path = "invocation_runtime/completion.rs"]
mod completion;

#[path = "invocation_runtime/fixture.rs"]
mod fixture;

#[path = "invocation_runtime/graph.rs"]
mod graph;

#[path = "invocation_runtime/owned_births.rs"]
mod owned_births;

#[path = "invocation_runtime/schema.rs"]
mod schema;

#[path = "cleanup_wake_private/observations.rs"]
mod prior_observations;

#[path = "queued_cleanup_private/loader.rs"]
mod loader;

#[path = "queued_cleanup_private/closure.rs"]
mod closure;

#[path = "queued_cleanup_private/observations.rs"]
mod observations;

#[path = "queued_cleanup_private/controller.rs"]
mod controller;

#[path = "queued_cleanup_private/controller_repository.rs"]
mod controller_repository;

#[path = "queued_cleanup_private/cleanup_port.rs"]
mod cleanup_port;

#[path = "queued_cleanup_private/admission.rs"]
mod admission;

#[path = "queued_cleanup_private/candidate.rs"]
mod candidate;

#[path = "queued_cleanup_private/completion.rs"]
mod queued_completion;

#[path = "queued_cleanup_private/withdrawal.rs"]
mod withdrawal;

#[path = "queued_cleanup_private/pins.rs"]
mod pins;

#[path = "queued_cleanup_private/history.rs"]
mod history;

#[path = "queued_cleanup_private/execution.rs"]
mod execution;

#[path = "queued_cleanup_private/closure_race.rs"]
mod closure_race;

#[path = "queued_cleanup_private/exclusion.rs"]
mod exclusion;

#[path = "queued_cleanup_private/locks.rs"]
mod locks;

#[path = "queued_cleanup_private/recovery.rs"]
mod recovery;

#[path = "queued_cleanup_private/cancel.rs"]
mod cancel;

#[path = "queued_cleanup_private/successor.rs"]
mod successor;

#[path = "queued_cleanup_private/fixture.rs"]
mod queued_fixture;
