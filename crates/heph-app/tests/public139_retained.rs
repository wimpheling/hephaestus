//! Genuine public139 producer/actor contracts with actual Local ext4 custody.
//! Requires configured native owned libkrun resources; no guest is started.
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Reuse established authenticated principals; pool helpers are unused.
mod support;
mod public139_retained {
    mod atomic;
    mod birth;
    mod fixture;
    mod lifecycle;
    mod native;
    mod pins;
    mod tests;
}
