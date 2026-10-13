//! Ordinary cleanup with actual public-schema metadata and filesystem effects.
// Reuse immutable seed helpers only; their unused helper surface has no cases.
// Reused support also reexports functions unused by these four cases.
#[allow(dead_code, unused_imports)]
#[path = "../../heph-std/secret/postgres/tests/postgres/support/mod.rs"]
mod support;

#[path = "secret_cleanup_dispatch/cases.rs"]
mod cases;
#[path = "secret_cleanup_dispatch/fixture.rs"]
mod fixture;
#[path = "secret_cleanup_dispatch/ports.rs"]
mod ports;
