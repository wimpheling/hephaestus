use std::{
    ffi::OsStr,
    io,
    path::Path,
    process::{Command, ExitStatus, Stdio},
};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, DevError>;

#[derive(Debug, Error)]
pub enum DevError {
    #[error("{0}")]
    Invalid(String),
    #[error("I/O operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("command `{program}` failed with {status}")]
    Command { program: String, status: ExitStatus },
    #[error("development supervisor is active; stop it with Ctrl-C before changing state")]
    SupervisorActive,
}

pub fn run(command: &mut Command) -> Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(DevError::Command { program, status })
    }
}

pub fn run_silent(command: &mut Command) -> Result<()> {
    command.stdout(Stdio::null()).stderr(Stdio::null());
    run(command)
}

pub fn run_quiet(program: &str, arguments: &[&str]) -> Result<bool> {
    let status = Command::new(program)
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

pub fn output(program: &str, arguments: &[&str]) -> Result<String> {
    let result = Command::new(program).args(arguments).output()?;
    if !result.status.success() {
        return Err(DevError::Command {
            program: program.into(),
            status: result.status,
        });
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

pub fn command_exists(program: &str) -> bool {
    Command::new("sh")
        .args(["-c", "command -v \"$1\" >/dev/null 2>&1", "sh", program])
        .status()
        .is_ok_and(|status| status.success())
}

pub fn remove_path(path: &Path) -> Result<()> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            if let Err(error) = std::fs::remove_dir_all(path) {
                remove_podman_owned_path(path, error)?;
            }
        }
        Ok(_) => {
            if let Err(error) = std::fs::remove_file(path) {
                remove_podman_owned_path(path, error)?;
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn remove_podman_owned_path(path: &Path, error: io::Error) -> Result<()> {
    if error.kind() != io::ErrorKind::PermissionDenied {
        return Err(error.into());
    }
    // Rootfs exports retain container-root ownership; Podman's user namespace
    // is the narrow cleanup boundary that can remove those files safely.
    let status = Command::new("podman")
        .args(["unshare", "find"])
        .arg(path)
        .args(["-depth", "-delete"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(DevError::Command {
            program: "podman".into(),
            status,
        })
    }
}

pub fn directory_size(path: &Path) -> u64 {
    let Ok(metadata) = path.symlink_metadata() else {
        return 0;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return metadata.len();
    }
    let Ok(entries) = path.read_dir() else {
        return 0;
    };
    entries
        .filter_map(std::result::Result::ok)
        .map(|entry| directory_size(&entry.path()))
        .sum()
}

pub fn path_argument(path: &Path) -> &OsStr {
    path.as_os_str()
}
