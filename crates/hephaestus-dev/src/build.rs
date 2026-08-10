use crate::{
    cli::BuildSelection,
    context::{DevContext, ELIXIR_IMAGE, GUEST_TARGET},
    process::{Result, run},
};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

pub fn build(context: &DevContext, selection: &BuildSelection) -> Result<()> {
    if selection.runtime() {
        build_runtime(context)?;
    }
    if selection.daemon() {
        build_daemon(context)?;
    }
    if selection.web() {
        build_web(context)?;
    }
    Ok(())
}

fn build_runtime(context: &DevContext) -> Result<()> {
    println!("building VM runtime and guest bootstrap");
    run(Command::new("rustup")
        .args(["target", "add", GUEST_TARGET])
        .current_dir(&context.repository_root))?;
    run(Command::new("cargo")
        .args([
            "build",
            "--release",
            "--package",
            "vm-libkrun",
            "--bin",
            "heph-init",
            "--target",
            GUEST_TARGET,
        ])
        .current_dir(&context.repository_root))?;
    run(Command::new("cargo")
        .args([
            "build",
            "--package",
            "vm-libkrun",
            "--bin",
            "hephaestus-vm-libkrun-worker",
        ])
        .current_dir(&context.repository_root))?;
    install_guest_bootstrap(context)
}

fn build_daemon(context: &DevContext) -> Result<()> {
    println!("building daemon and development support binaries");
    run(Command::new("cargo")
        .args(["build", "--package", "hephaestus-app", "--bins"])
        .current_dir(&context.repository_root))
}

fn build_web(context: &DevContext) -> Result<()> {
    println!("building Phoenix and web assets");
    let mount = format!("{}:/workspace:z", context.repository_root.display());
    run(Command::new("podman")
        .args([
            "run",
            "--rm",
            "--network",
            "host",
            "--volume",
            &mount,
            "--workdir",
            "/workspace/web",
            "--env",
            "MIX_ENV=dev",
            ELIXIR_IMAGE,
            "sh",
            "-lc",
            "mix local.hex --force >/dev/null && mix deps.get && mix assets.setup && mix assets.build",
        ])
        .current_dir(&context.repository_root))
}

pub fn install_guest_bootstrap(context: &DevContext) -> Result<()> {
    let cache = context.image_cache();
    if !cache.is_dir() {
        return Ok(());
    }
    let source = context
        .repository_root
        .join("target")
        .join(GUEST_TARGET)
        .join("release/heph-init");
    for entry in fs::read_dir(cache)? {
        let entry = entry?;
        let image = entry.path();
        let metadata = fs::symlink_metadata(&image)?;
        let marker = image.join(".hephaestus-image");
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || fs::symlink_metadata(&marker).is_ok_and(|metadata| metadata.file_type().is_symlink())
            || !marker.is_file()
        {
            continue;
        }
        let destination = image.join("usr/libexec/hephaestus/heph-init");
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &destination)?;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}
