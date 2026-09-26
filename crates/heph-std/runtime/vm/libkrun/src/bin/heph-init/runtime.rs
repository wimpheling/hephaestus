use serde::Serialize;
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt, chown},
    path::Path,
};
use vm_libkrun::protocol::{
    GUEST_RUNTIME_AUTHORITY_PATH, GuestCommandMessage, RUNTIME_AUTHORITY_PATH_ENV,
    RuntimeAuthorityMessage,
};
use zeroize::Zeroizing;

use crate::{
    AGENT_GID, AGENT_UID, GUEST_COMMAND_OPEN_FILES, PLATFORM_OCI_BUILDER_ENV,
    PLATFORM_OCI_BUILDER_PROGRAM, PLATFORM_OCI_VERIFIER_ENV, PLATFORM_OCI_VERIFIER_PROGRAM,
    RUNTIME_AUTHORITY_DIRECTORY,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformOciOperation {
    None,
    Builder,
    Verifier,
}

impl PlatformOciOperation {
    pub(crate) const fn runs_as_root(self) -> bool {
        matches!(self, Self::Builder)
    }

    pub(crate) const fn needs_large_fd_limit(self) -> bool {
        !matches!(self, Self::None)
    }
}

pub fn platform_oci_operation(command: &GuestCommandMessage) -> PlatformOciOperation {
    if command.program == PLATFORM_OCI_BUILDER_PROGRAM
        && command
            .env
            .get(PLATFORM_OCI_BUILDER_ENV)
            .is_some_and(|value| value == "1")
    {
        PlatformOciOperation::Builder
    } else if command.program == PLATFORM_OCI_VERIFIER_PROGRAM
        && command
            .env
            .get(PLATFORM_OCI_VERIFIER_ENV)
            .is_some_and(|value| value == "1")
    {
        PlatformOciOperation::Verifier
    } else {
        PlatformOciOperation::None
    }
}

#[allow(unsafe_code)]
pub fn provision_guest_open_files() -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: GUEST_COMMAND_OPEN_FILES,
        rlim_max: GUEST_COMMAND_OPEN_FILES,
    };
    // SAFETY: `limit` is initialized, and this bounded root-only bootstrap
    // runs before the unprivileged build command is spawned.
    let result = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[derive(Serialize)]
struct GuestRuntimeAuthority<'a> {
    session_id: uuid::Uuid,
    generation: u64,
    credential_hex: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_git_credential: Option<&'a str>,
}

pub fn persist_runtime_authority(
    authority: &RuntimeAuthorityMessage,
    command: &GuestCommandMessage,
) -> Result<(uuid::Uuid, u64), Box<dyn std::error::Error + Send + Sync>> {
    if authority.generation == 0 {
        return Err("runtime authority generation must be positive".into());
    }
    let directory = Path::new(RUNTIME_AUTHORITY_DIRECTORY);
    fs::create_dir_all(directory)
        .map_err(|error| format!("create authority directory: {error}"))?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("seal authority directory: {error}"))?;
    chown(directory, Some(AGENT_UID), Some(AGENT_GID))
        .map_err(|error| format!("own authority directory: {error}"))?;

    let mut credential_hex = Zeroizing::new(String::with_capacity(authority.credential.len() * 2));
    for byte in &authority.credential {
        use std::fmt::Write as _;
        write!(&mut credential_hex, "{byte:02x}")?;
    }
    let mut runtime_git_credential = authority.runtime_git_credential.as_ref().map(|credential| {
        let mut encoded = String::with_capacity(credential.len() * 2);
        for byte in credential {
            use std::fmt::Write as _;
            // Writing to a String cannot fail.
            let _ = write!(&mut encoded, "{byte:02x}");
        }
        Zeroizing::new(encoded)
    });
    let document = GuestRuntimeAuthority {
        session_id: authority.session_id,
        generation: authority.generation,
        credential_hex: credential_hex.as_str(),
        runtime_git_credential: runtime_git_credential
            .as_deref()
            .map(std::string::String::as_str),
    };
    let bytes = Zeroizing::new(serde_json::to_vec(&document)?);
    let credential_path = command
        .env
        .get(RUNTIME_AUTHORITY_PATH_ENV)
        .map_or_else(|| Path::new(GUEST_RUNTIME_AUTHORITY_PATH), Path::new);
    if credential_path.parent() != Some(directory) || credential_path.extension().is_none() {
        return Err("runtime authority credential path is invalid".into());
    }
    match fs::symlink_metadata(credential_path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(credential_path)
                .map_err(|error| format!("remove stale authority credential: {error}"))?;
        }
        Ok(_) => return Err("stale authority credential is not a regular file".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect authority credential: {error}").into()),
    }
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o400)
        .open(credential_path)
        .map_err(|error| format!("create authority credential: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("write authority credential: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync authority credential: {error}"))?;
    chown(credential_path, Some(AGENT_UID), Some(AGENT_GID))?;
    if let Some(credential) = &mut runtime_git_credential {
        credential.clear();
    }
    Ok((authority.session_id, authority.generation))
}
