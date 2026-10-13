//! Actual scoped release callback SQL; no physical VM or authenticated HTTP proof.

#[path = "legacy_completion/fixture.rs"]
mod fixture;
#[path = "legacy_completion/normal.rs"]
mod normal;
#[path = "legacy_completion/update.rs"]
mod update;
// The existing exact release graph also contains unrelated retry helpers.
#[path = "../../../../run/postgres/tests/postgres/support.rs"]
#[allow(dead_code)]
mod support;
// Reuse immutable candidate and actor setup; no RunPG adapter dependency.
#[path = "../../../../run/postgres/tests/planned_creation/seed.rs"]
#[allow(dead_code)]
mod seed;
