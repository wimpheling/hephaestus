//! Private profile foundation: database correlation and fencing, never VM proof.

#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Reuse the existing real cross-project graph and fixture type.
mod support;
mod run_profiles_private {
    mod cases;
    mod fixture;
    mod history;
    mod producers;
}
