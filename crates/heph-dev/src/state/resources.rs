//! Development state selection, lifecycle, and filesystem resources.

use super::{database, images};
use crate::{
    cache::format_bytes,
    cli::{STATE_RESOURCES, StateResource, StateSelection},
    context::DevContext,
    process::{DevError, Result, directory_size, remove_path, run_quiet, run_silent},
    zot,
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

pub(super) fn list(context: &DevContext) -> Result<()> {
    for resource in STATE_RESOURCES {
        match resource {
            StateResource::Postgresql => print_volume("postgresql", &context.postgres_volume())?,
            StateResource::Nats => print_volume("nats", &context.nats_volume())?,
            StateResource::Zot => print_path("Zot registry", &context.zot_root()),
            StateResource::Runtime => {
                print_path("VM runtime", &context.runtime_root);
                print_path("secret runtime", &context.secret_runtime_root);
            }
            _ => {
                let path = resource_path(context, resource);
                print_path(resource_name(resource), &path);
            }
        }
    }
    Ok(())
}

pub(super) fn init(context: &DevContext, selection: &StateSelection) -> Result<()> {
    ensure_supervisor_inactive(context)?;
    let fixtures = selection.selected(StateResource::Fixtures);
    if selection.selected(StateResource::Postgresql) || fixtures {
        create_volume(&context.postgres_volume())?;
    }
    if selection.selected(StateResource::Nats) {
        create_volume(&context.nats_volume())?;
    }
    if selection.selected(StateResource::Zot) {
        zot::initialize(context)?;
    }
    for resource in [
        StateResource::Repositories,
        StateResource::Artifacts,
        StateResource::AgentVolumes,
        StateResource::Workspaces,
        StateResource::Logs,
    ] {
        if selection.selected(resource) || (fixtures && fixture_uses(resource)) {
            fs::create_dir_all(resource_path(context, resource))?;
        }
    }
    if selection.selected(StateResource::SecretKeys) {
        initialize_secret_keys(context)?;
    }
    if selection.selected(StateResource::Runtime) {
        initialize_runtime(context)?;
    }
    if selection.selected(StateResource::Rootfs) {
        images::initialize_image_cache(context)?;
    }
    if fixtures {
        database::initialize_fixtures(context)?;
    } else if selection.selected(StateResource::Postgresql) {
        database::initialize_database(context, true)?;
    }
    println!("selected development state initialized");
    Ok(())
}

pub(super) fn clean(context: &DevContext, selection: &StateSelection) -> Result<()> {
    ensure_supervisor_inactive(context)?;
    let fixtures = selection.selected(StateResource::Fixtures);
    if selection.selected(StateResource::Postgresql) || fixtures {
        let _ignored = run_silent(Command::new("podman").args([
            "rm",
            "--force",
            &context.postgres_container(),
        ]));
        remove_volume(&context.postgres_volume())?;
    }
    if selection.selected(StateResource::Nats) {
        let _ignored =
            run_silent(Command::new("podman").args(["rm", "--force", &context.nats_container()]));
        remove_volume(&context.nats_volume())?;
    }
    if selection.selected(StateResource::Zot) {
        zot::clean(context)?;
    }
    for resource in [
        StateResource::Repositories,
        StateResource::Artifacts,
        StateResource::AgentVolumes,
        StateResource::Workspaces,
    ] {
        if selection.selected(resource) || (fixtures && fixture_uses(resource)) {
            remove_path(&resource_path(context, resource))?;
        }
    }
    if selection.selected(StateResource::SecretKeys) {
        remove_path(&resource_path(context, StateResource::SecretKeys))?;
    }
    if selection.selected(StateResource::Rootfs) {
        for reference in images::configured_oci_images()? {
            let digest = images::image_digest(&reference)?;
            let _ignored = run_silent(Command::new("podman").args([
                "rm",
                "--force",
                &context.image_container(digest),
            ]));
        }
        remove_path(&resource_path(context, StateResource::Rootfs))?;
    }
    if selection.selected(StateResource::Runtime) {
        cleanup_recorded_cgroup(context)?;
        remove_path(&context.runtime_root)?;
        remove_path(&context.secret_runtime_root)?;
        remove_path(&context.local_root.join("cgroup.path"))?;
        remove_path(&context.local_root.join("daemon.pid"))?;
        remove_path(&context.local_root.join("oidc.pid"))?;
        remove_path(&context.local_root.join("web-log.pid"))?;
        remove_path(&context.supervisor_pid_file())?;
    }
    if selection.selected(StateResource::Logs) {
        remove_path(&context.logs())?;
    }
    if fixtures {
        remove_path(&context.seed_file())?;
    }
    println!("selected development state cleaned");
    Ok(())
}

pub(super) fn reinit(context: &DevContext, selection: &StateSelection) -> Result<()> {
    clean(context, selection)?;
    init(context, selection)
}

pub(super) fn ensure_supervisor_inactive(context: &DevContext) -> Result<()> {
    let pid_file = context.supervisor_pid_file();
    let Ok(pid) = fs::read_to_string(&pid_file) else {
        return Ok(());
    };
    let pid = pid.trim();
    if !pid.is_empty() && run_quiet("kill", &["-0", pid])? {
        return Err(DevError::SupervisorActive);
    }
    remove_path(&pid_file)
}

pub(super) const fn fixture_uses(resource: StateResource) -> bool {
    matches!(
        resource,
        StateResource::Repositories
            | StateResource::Artifacts
            | StateResource::AgentVolumes
            | StateResource::Workspaces
    )
}

pub(super) const fn resource_name(resource: StateResource) -> &'static str {
    match resource {
        StateResource::Postgresql => "postgresql",
        StateResource::Nats => "nats",
        StateResource::Zot => "Zot registry",
        StateResource::Repositories => "repositories",
        StateResource::Artifacts => "artifacts",
        StateResource::AgentVolumes => "agent volumes",
        StateResource::Workspaces => "workspaces",
        StateResource::SecretKeys => "secret keys",
        StateResource::Rootfs => "OCI image cache",
        StateResource::Fixtures => "fixtures",
        StateResource::Runtime => "runtime",
        StateResource::Logs => "logs",
    }
}

pub(super) fn resource_path(context: &DevContext, resource: StateResource) -> PathBuf {
    match resource {
        StateResource::Postgresql | StateResource::Nats => context.local_root.clone(),
        StateResource::Zot => context.zot_root(),
        StateResource::Repositories => context.local_root.join("repositories"),
        StateResource::Artifacts => context.local_root.join("artifacts"),
        StateResource::AgentVolumes => context.local_root.join("volumes"),
        StateResource::Workspaces => context.local_root.join("workspaces"),
        StateResource::SecretKeys => context.local_root.join("secret-keys"),
        StateResource::Rootfs => context.image_cache(),
        StateResource::Fixtures => context.seed_file(),
        StateResource::Runtime => context.runtime_root.clone(),
        StateResource::Logs => context.logs(),
    }
}

pub(super) fn print_path(label: &str, path: &Path) {
    println!(
        "{label:16} {:>10}  {}",
        format_bytes(directory_size(path)),
        if path.exists() {
            path.display().to_string()
        } else {
            format!("missing ({})", path.display())
        }
    );
}

pub(super) fn print_volume(label: &str, volume: &str) -> Result<()> {
    let exists = run_quiet("podman", &["volume", "exists", volume])?;
    println!(
        "{label:16} {:>10}  podman volume {volume}",
        if exists { "present" } else { "missing" }
    );
    Ok(())
}

pub(super) fn create_volume(volume: &str) -> Result<()> {
    if !run_quiet("podman", &["volume", "exists", volume])? {
        run_silent(Command::new("podman").args(["volume", "create", volume]))?;
    }
    Ok(())
}

pub(super) fn remove_volume(volume: &str) -> Result<()> {
    if run_quiet("podman", &["volume", "exists", volume])? {
        run_silent(Command::new("podman").args(["volume", "rm", "--force", volume]))?;
    }
    Ok(())
}

fn initialize_secret_keys(context: &DevContext) -> Result<()> {
    let directory = resource_path(context, StateResource::SecretKeys);
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    let key = directory.join("local-v1");
    if key.exists() {
        return Ok(());
    }
    let mut random = fs::File::open("/dev/urandom")?;
    let mut bytes = [0_u8; 32];
    random.read_exact(&mut bytes)?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(&key)?;
    output.write_all(&bytes)?;
    fs::set_permissions(key, fs::Permissions::from_mode(0o400))?;
    Ok(())
}

fn initialize_runtime(context: &DevContext) -> Result<()> {
    for directory in [&context.runtime_root, &context.secret_runtime_root] {
        fs::create_dir_all(directory)?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn cleanup_recorded_cgroup(context: &DevContext) -> Result<()> {
    let path_file = context.local_root.join("cgroup.path");
    let Ok(recorded) = fs::read_to_string(&path_file) else {
        return Ok(());
    };
    let cgroup = PathBuf::from(recorded.trim());
    let valid = cgroup.starts_with("/sys/fs/cgroup")
        && cgroup
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("hephaestus-local-"));
    if !valid {
        return Err(DevError::Invalid(format!(
            "refusing to clean unexpected recorded cgroup {}",
            cgroup.display()
        )));
    }
    if cgroup.join("cgroup.kill").exists() {
        let _ignored = fs::write(cgroup.join("cgroup.kill"), "1\n");
    }
    for _attempt in 0..100 {
        let populated = fs::read_to_string(cgroup.join("cgroup.events"))
            .is_ok_and(|events| events.lines().any(|line| line == "populated 1"));
        if !populated {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let mut descendants = Vec::new();
    collect_directories(&cgroup, &mut descendants);
    descendants.sort_unstable_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in descendants {
        let _ignored = fs::remove_dir(directory);
    }
    let _ignored = fs::remove_dir(cgroup);
    Ok(())
}

fn collect_directories(root: &Path, directories: &mut Vec<PathBuf>) {
    let Ok(entries) = root.read_dir() else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            directories.push(path.clone());
            collect_directories(&path, directories);
        }
    }
}
