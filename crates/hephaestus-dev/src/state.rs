use crate::{
    build,
    cache::format_bytes,
    cli::{BuildSelection, STATE_RESOURCES, StateResource, StateSelection},
    context::{DevContext, FEDORA_IMAGE, POSTGRES_IMAGE},
    process::{
        DevError, Result, directory_size, path_argument, remove_path, run, run_quiet, run_silent,
    },
    zot,
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

pub fn list(context: &DevContext) -> Result<()> {
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

pub fn init(context: &DevContext, selection: &StateSelection) -> Result<()> {
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
        initialize_rootfs(context)?;
    }
    if fixtures {
        initialize_fixtures(context)?;
    } else if selection.selected(StateResource::Postgresql) {
        initialize_database(context, true)?;
    }
    println!("selected development state initialized");
    Ok(())
}

pub fn clean(context: &DevContext, selection: &StateSelection) -> Result<()> {
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
        let _ignored =
            run_silent(Command::new("podman").args(["rm", "--force", &context.rootfs_container()]));
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

pub fn reinit(context: &DevContext, selection: &StateSelection) -> Result<()> {
    clean(context, selection)?;
    init(context, selection)
}

fn ensure_supervisor_inactive(context: &DevContext) -> Result<()> {
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

const fn fixture_uses(resource: StateResource) -> bool {
    matches!(
        resource,
        StateResource::Repositories
            | StateResource::Artifacts
            | StateResource::AgentVolumes
            | StateResource::Workspaces
    )
}

const fn resource_name(resource: StateResource) -> &'static str {
    match resource {
        StateResource::Postgresql => "postgresql",
        StateResource::Nats => "nats",
        StateResource::Zot => "Zot registry",
        StateResource::Repositories => "repositories",
        StateResource::Artifacts => "artifacts",
        StateResource::AgentVolumes => "agent volumes",
        StateResource::Workspaces => "workspaces",
        StateResource::SecretKeys => "secret keys",
        StateResource::Rootfs => "rootfs",
        StateResource::Fixtures => "fixtures",
        StateResource::Runtime => "runtime",
        StateResource::Logs => "logs",
    }
}

fn resource_path(context: &DevContext, resource: StateResource) -> PathBuf {
    match resource {
        StateResource::Postgresql | StateResource::Nats => context.local_root.clone(),
        StateResource::Zot => context.zot_root(),
        StateResource::Repositories => context.local_root.join("repositories"),
        StateResource::Artifacts => context.local_root.join("artifacts"),
        StateResource::AgentVolumes => context.local_root.join("volumes"),
        StateResource::Workspaces => context.local_root.join("workspaces"),
        StateResource::SecretKeys => context.local_root.join("secret-keys"),
        StateResource::Rootfs => context.local_root.join("root-images"),
        StateResource::Fixtures => context.seed_file(),
        StateResource::Runtime => context.runtime_root.clone(),
        StateResource::Logs => context.logs(),
    }
}

fn print_path(label: &str, path: &Path) {
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

fn print_volume(label: &str, volume: &str) -> Result<()> {
    let exists = run_quiet("podman", &["volume", "exists", volume])?;
    println!(
        "{label:16} {:>10}  podman volume {volume}",
        if exists { "present" } else { "missing" }
    );
    Ok(())
}

fn create_volume(volume: &str) -> Result<()> {
    if !run_quiet("podman", &["volume", "exists", volume])? {
        run_silent(Command::new("podman").args(["volume", "create", volume]))?;
    }
    Ok(())
}

fn remove_volume(volume: &str) -> Result<()> {
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

fn initialize_rootfs(context: &DevContext) -> Result<()> {
    build::build(context, &BuildSelection::runtime_only())?;
    let root_image = context.root_image();
    if !root_image.join(".hephaestus-image").is_file() {
        import_rootfs(context, &root_image)?;
    }
    build::install_guest_bootstrap(context)
}

fn import_rootfs(context: &DevContext, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| DevError::Invalid("root image has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".fedora-minimal.{}", std::process::id()));
    remove_path(&staging)?;
    fs::create_dir(&staging)?;
    run(Command::new("podman").args(["pull", FEDORA_IMAGE]))?;
    let _ignored =
        run_silent(Command::new("podman").args(["rm", "--force", &context.rootfs_container()]));
    run_silent(Command::new("podman").args([
        "create",
        "--name",
        &context.rootfs_container(),
        FEDORA_IMAGE,
        "/bin/true",
    ]))?;
    let import_result = export_container(&context.rootfs_container(), &staging)
        .and_then(|()| configure_guest_identity(&staging))
        .and_then(|()| {
            fs::write(staging.join(".hephaestus-image"), FEDORA_IMAGE)?;
            fs::rename(&staging, destination)?;
            Ok(())
        });
    let _ignored =
        run_silent(Command::new("podman").args(["rm", "--force", &context.rootfs_container()]));
    if import_result.is_err() {
        let _ignored = remove_path(&staging);
    }
    import_result
}

fn export_container(container: &str, destination: &Path) -> Result<()> {
    let mut exporter = Command::new("podman")
        .args(["export", container])
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = exporter
        .stdout
        .take()
        .ok_or_else(|| DevError::Invalid("podman export did not expose stdout".into()))?;
    let tar_status = Command::new("tar")
        .arg("-C")
        .arg(path_argument(destination))
        .args(["-xf", "-"])
        .stdin(stdout)
        .status()?;
    let export_status = exporter.wait()?;
    if !tar_status.success() {
        return Err(DevError::Command {
            program: "tar".into(),
            status: tar_status,
        });
    }
    if !export_status.success() {
        return Err(DevError::Command {
            program: "podman".into(),
            status: export_status,
        });
    }
    Ok(())
}

fn configure_guest_identity(root: &Path) -> Result<()> {
    let passwd = root.join("etc/passwd");
    let group = root.join("etc/group");
    if contains_numeric_identity(&passwd, 2, "10001")?
        || contains_numeric_identity(&group, 2, "10001")?
    {
        return Err(DevError::Invalid(
            "the pinned root image already assigns guest UID/GID 10001".into(),
        ));
    }
    OpenOptions::new()
        .append(true)
        .open(passwd)?
        .write_all(b"heph-agent:x:10001:10001:Hephaestus agent:/nonexistent:/sbin/nologin\n")?;
    OpenOptions::new()
        .append(true)
        .open(group)?
        .write_all(b"heph-agent:x:10001:\n")?;
    Ok(())
}

fn contains_numeric_identity(path: &Path, field: usize, expected: &str) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    Ok(content
        .lines()
        .any(|line| line.split(':').nth(field) == Some(expected)))
}

fn initialize_fixtures(context: &DevContext) -> Result<()> {
    fs::create_dir_all(context.local_root.join("repositories"))?;
    fs::create_dir_all(context.local_root.join("artifacts"))?;
    initialize_database(context, false)
}

fn initialize_database(context: &DevContext, schema_only: bool) -> Result<()> {
    create_volume(&context.postgres_volume())?;
    let _ignored =
        run_silent(Command::new("podman").args(["rm", "--force", &context.postgres_container()]));
    let volume = format!("{}:/var/lib/postgresql/data", context.postgres_volume());
    run_silent(Command::new("podman").args([
        "run",
        "--detach",
        "--rm",
        "--name",
        &context.postgres_container(),
        "--env",
        "POSTGRES_PASSWORD=postgres",
        "--env",
        "POSTGRES_DB=hephaestus",
        "--publish",
        &format!("127.0.0.1:{}:5432", context.postgres_port),
        "--volume",
        &volume,
        POSTGRES_IMAGE,
    ]))?;
    let result = wait_for_postgres(context)
        .and_then(|()| build::build(context, &BuildSelection::daemon_only()))
        .and_then(|()| run_seed(context, schema_only));
    let _ignored =
        run_silent(Command::new("podman").args(["rm", "--force", &context.postgres_container()]));
    result
}

fn wait_for_postgres(context: &DevContext) -> Result<()> {
    for _attempt in 0..600 {
        if run_quiet(
            "podman",
            &[
                "exec",
                &context.postgres_container(),
                "pg_isready",
                "--quiet",
                "--username",
                "postgres",
                "--dbname",
                "hephaestus",
            ],
        )? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(DevError::Invalid(
        "timed out waiting for fixture PostgreSQL".into(),
    ))
}

fn run_seed(context: &DevContext, schema_only: bool) -> Result<()> {
    let database_url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/hephaestus?sslmode=disable",
        context.postgres_port
    );
    let mut command = Command::new(
        context
            .repository_root
            .join("target/debug/hephaestus-e2e-seed"),
    );
    if schema_only {
        command.arg("--schema-only");
    }
    let result = command
        .env("HEPHAESTUS_DATABASE_URL", database_url)
        .env(
            "HEPHAESTUS_REPOSITORY_ROOT",
            context.local_root.join("repositories"),
        )
        .env(
            "HEPHAESTUS_ARTIFACT_ROOT",
            context.local_root.join("artifacts"),
        )
        .env("HEPHAESTUS_BROWSER_OIDC_ISSUER", "http://127.0.0.1:5556")
        .output()?;
    if !result.status.success() {
        return Err(DevError::Command {
            program: "hephaestus-e2e-seed".into(),
            status: result.status,
        });
    }
    if !schema_only {
        fs::create_dir_all(&context.local_root)?;
        fs::write(context.seed_file(), result.stdout)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::contains_numeric_identity;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn detects_exact_numeric_identity_field() {
        let fixture = tempdir().expect("fixture");
        let passwd = fixture.path().join("passwd");
        fs::write(&passwd, "root:x:0:0\nagent:x:10001:10001\n").expect("write");
        assert!(contains_numeric_identity(&passwd, 2, "10001").expect("read"));
        assert!(!contains_numeric_identity(&passwd, 2, "1000").expect("read"));
    }
}
