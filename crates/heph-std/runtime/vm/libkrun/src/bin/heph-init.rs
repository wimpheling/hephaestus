//! Minimal guest bootstrap for approved Hephaestus libkrun images.

use std::io::{self, Write};
use std::time::Duration;

#[path = "heph-init/bootstrap.rs"]
mod bootstrap;
#[path = "heph-init/gateway.rs"]
mod gateway;
#[path = "heph-init/git_bridge.rs"]
mod git_bridge;
#[path = "heph-init/mounts.rs"]
mod mounts;
#[path = "heph-init/protocol_io.rs"]
mod protocol_io;
#[path = "heph-init/runtime.rs"]
mod runtime;
#[path = "heph-init/service.rs"]
mod service;
#[cfg(test)]
#[path = "heph-init/tests.rs"]
mod tests;
#[path = "heph-init/vsock.rs"]
mod vsock;

const CONTROL_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const PLATFORM_OCI_BUILDER_ENV: &str = "HEPH_PLATFORM_OCI_BUILDER";
const PLATFORM_OCI_BUILDER_PROGRAM: &str = "/usr/libexec/hephaestus/oci-build";
const PLATFORM_OCI_VERIFIER_ENV: &str = "HEPH_PLATFORM_OCI_VERIFIER";
const PLATFORM_OCI_VERIFIER_PROGRAM: &str = "/usr/libexec/hephaestus/oci-verify";
const GUEST_COMMAND_OPEN_FILES: libc::rlim_t = 65_536;
const BUILD_GUEST_ENV: &str = "HEPH_BUILD_GUEST";
const AGENT_UID: u32 = 10_001;
const AGENT_GID: u32 = 10_001;
const RUNTIME_AUTHORITY_DIRECTORY: &str = "/run/hephaestus-authority";

pub(crate) use gateway::gateway_handler_loop;
#[cfg(test)]
pub(crate) use mounts::find_ext4_device_in;
pub(crate) use mounts::{
    connect_control, mount_state_volume, mount_virtiofs, send_guest_error, unmount,
};
pub(crate) use protocol_io::{
    exit_parts, handle_host_messages, join_log_thread, lock, pump_logs, read_frame, signal_process,
    wait_command, write_frame, write_message,
};
#[cfg(test)]
pub(crate) use runtime::PlatformOciOperation;
pub(crate) use runtime::{
    persist_runtime_authority, platform_oci_operation, provision_guest_open_files,
};

fn main() {
    if let Err(error) = bootstrap::run() {
        let _write_result = writeln!(io::stderr(), "heph-init: {error}");
        std::process::exit(125);
    }
}
