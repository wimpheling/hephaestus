//! Actual accepted mailbox producer profiles; no delegation or physical proof.

#[path = "postgres/support.rs"]
// Reuse the real guarded instance/envelope fixture; recovery helpers are unused.
#[allow(dead_code)]
mod fixture;
mod execution_profiles {
    mod claims;
    mod support;
}
