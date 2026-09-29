use super::receipt::TOOL_IMAGE;
use crate::{
    context::DevContext,
    process::{DevError, Result, directory_size, run},
};
use std::{fs, process::Command};

pub(super) fn build_release_tools(context: &DevContext) -> Result<()> {
    run(Command::new("cargo")
        .args([
            "build",
            "-p",
            "registry-release",
            "--bin",
            "hephaestus-registry-release",
            "-p",
            "bootstrap-postgres",
            "--bin",
            "hephaestus-operator",
        ])
        .current_dir(&context.repository_root))?;
    Ok(())
}

pub(super) fn binary_version(binary: &std::path::Path) -> Result<String> {
    let output = Command::new(binary).arg("--version").output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(DevError::Command {
            program: binary.display().to_string(),
            status: output.status,
        })
    }
}

pub(super) fn build_tool_image(context: &DevContext) -> Result<()> {
    let tool_context = context.repository_root.join("platform/build-tools");
    println!("building pinned platform-image tool container");
    run(Command::new("podman")
        .args(["build", "--pull=never", "--tag", TOOL_IMAGE])
        .arg(tool_context)
        .current_dir(&context.repository_root))
}

pub(super) fn create_volume(volume: &str) -> Result<()> {
    let exists = Command::new("podman")
        .args(["volume", "exists", volume])
        .status()?
        .success();
    if !exists {
        run(Command::new("podman").args(["volume", "create", volume]))?;
    }
    Ok(())
}

pub(super) fn print_directory(label: &str, path: &std::path::Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            println!(
                "{label:24} {:>10}  {}",
                directory_size(path),
                path.display()
            );
            Ok(())
        }
        Ok(_) => Err(DevError::Invalid(format!(
            "{label} must be a non-symlink directory: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("{label:24} {:>10}  missing ({})", 0, path.display());
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
