//! Host-owned `pre-receive` entry point for exact-runtime Git capabilities.

use git_capability_domain::RepositoryId;
use git_http::{
    receive_hook::{ReceiveHookInput, authorize_quarantined_receive},
    receive_policy::ResolvedRuntimeReceiveContext,
};
use std::{
    env, fs,
    io::{self, BufReader},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

const CONTEXT_ENV: &str = "HEPH_RUNTIME_RECEIVE_CONTEXT_FILE";
const REPOSITORY_ENV: &str = "HEPH_RUNTIME_RECEIVE_REPOSITORY";
const REQUEST_BYTES_ENV: &str = "HEPH_RUNTIME_RECEIVE_REQUEST_BYTES";
const MAX_CONTEXT_BYTES: usize = 512 * 1_024;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("runtime receive denied: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), HookEntryError> {
    let context_path = env::var_os(CONTEXT_ENV)
        .map(PathBuf::from)
        .ok_or(HookEntryError::InvalidHostContext)?;
    let metadata =
        fs::symlink_metadata(&context_path).map_err(|_| HookEntryError::InvalidHostContext)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != current_process_uid()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.len() > u64::try_from(MAX_CONTEXT_BYTES).unwrap_or(u64::MAX)
    {
        return Err(HookEntryError::InvalidHostContext);
    }
    let context_bytes = fs::read(context_path).map_err(|_| HookEntryError::InvalidHostContext)?;
    if context_bytes.len() > MAX_CONTEXT_BYTES {
        return Err(HookEntryError::InvalidHostContext);
    }
    let context = ResolvedRuntimeReceiveContext::from_hook_json(&context_bytes)
        .map_err(|_| HookEntryError::InvalidHostContext)?;
    let repository_id = env::var(REPOSITORY_ENV)
        .map_err(|_| HookEntryError::InvalidHostContext)?
        .parse::<RepositoryId>()
        .map_err(|_| HookEntryError::InvalidHostContext)?;
    let request_bytes = env::var(REQUEST_BYTES_ENV)
        .map_err(|_| HookEntryError::InvalidHostContext)?
        .parse::<u64>()
        .map_err(|_| HookEntryError::InvalidHostContext)?;
    let repository_path = env::var_os("GIT_DIR").map_or_else(|| PathBuf::from("."), PathBuf::from);
    let quarantine_path = env::var_os("GIT_QUARANTINE_PATH")
        .map(PathBuf::from)
        .ok_or(HookEntryError::InvalidHostContext)?;
    let now_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HookEntryError::InvalidHostClock)?
        .as_secs()
        .try_into()
        .map_err(|_| HookEntryError::InvalidHostClock)?;
    authorize_quarantined_receive(
        context,
        &ReceiveHookInput {
            repository_path: &repository_path,
            repository_id,
            quarantine_path: &quarantine_path,
            request_bytes,
            now_unix_seconds,
        },
        BufReader::new(io::stdin().lock()),
    )?;
    Ok(())
}

fn current_process_uid() -> u32 {
    // `/proc/self` is a trusted kernel view; its ownership gives the host
    // metadata ownership gives the effective host process UID without FFI.
    fs::metadata("/proc/self")
        .map(|metadata| metadata.uid())
        .unwrap_or(u32::MAX)
}

#[derive(Debug, thiserror::Error)]
enum HookEntryError {
    #[error("host receive context is invalid")]
    InvalidHostContext,
    #[error("host clock is invalid")]
    InvalidHostClock,
    #[error(transparent)]
    Receive(#[from] git_http::receive_hook::ReceiveHookError),
}
