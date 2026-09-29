//! Raw durable-storage checks for the disposable cooking database.

#[path = "confinement/logs.rs"]
mod logs;
#[path = "confinement/nats.rs"]
mod nats;
#[path = "confinement/patterns.rs"]
mod patterns;
#[path = "confinement/storage.rs"]
mod storage;
#[cfg(test)]
#[path = "confinement/tests.rs"]
mod tests;

pub(crate) use logs::{assert_build_logs_have_no_credentials, assert_vm_logs_have_no_credentials};
pub use nats::assert_nats_has_no_credentials;
pub use patterns::credential_patterns;
pub(crate) use patterns::{CREDENTIAL_PATTERNS, VmLogScan, assert_bytes_have_no_credentials};
pub use storage::{
    assert_database_has_no_credentials, assert_state_snapshot_has_no_credentials,
    assert_static_storage_scan_matches_catalog,
};
