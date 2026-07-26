//! Fedora/Linux microVM provider backed by libkrun, libkrunfw, KVM, and passt.
//!
//! The provider runs every VMM in a dedicated unprivileged worker process.
//! Construction and validation happen in the parent; provider-specific file
//! descriptors and processes remain owned by the worker.

mod cgroup;
mod config;
mod ffi;
mod framing;
mod network;
pub mod protocol;
mod provider;
mod validation;
mod worker;

pub use config::{CgroupLimits, IoLimit, LibkrunConfig};
pub use provider::LibkrunProvider;

/// Runs the dedicated libkrun worker process.
///
/// This entry point is used by the `hephaestus-vm-libkrun-worker` binary and is
/// not intended to be called inside the supervisor process.
///
/// # Errors
///
/// Returns an error when the worker arguments, IPC connection, configuration,
/// or backend initialization fail.
pub fn worker_main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    worker::main()
}
