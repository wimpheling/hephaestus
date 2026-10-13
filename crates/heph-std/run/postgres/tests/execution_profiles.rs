//! Explicit constructor profiles against public107 and private profile SQL.

#[path = "cleanup/fixture.rs"]
// Shared schema/roles helpers also contain unrelated cleanup scenarios.
#[allow(dead_code)]
mod fixture;
#[path = "execution_profiles/normal.rs"]
mod normal;
#[path = "execution_profiles/support.rs"]
mod profile_support;
#[path = "planned_creation/seed.rs"]
mod seed;
#[path = "postgres/support.rs"]
// Exact real provenance fixture; unrelated retry helpers are unused here.
#[allow(dead_code)]
mod support;
#[path = "execution_profiles/update.rs"]
mod update;
