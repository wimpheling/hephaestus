use crate::{
    build,
    cache::format_bytes,
    cli::{BuildSelection, STATE_RESOURCES, StateResource, StateSelection},
    context::{DEFAULT_LOCAL_OCI_IMAGE, DevContext, POSTGRES_IMAGE},
    process::{
        DevError, Result, directory_size, path_argument, remove_path, run, run_quiet, run_silent,
    },
    zot,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    env,
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
        initialize_image_cache(context)?;
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
        for reference in configured_oci_images()? {
            let digest = image_digest(&reference)?;
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
        StateResource::Rootfs => "OCI image cache",
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
        StateResource::Rootfs => context.image_cache(),
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

/// Materializes each configured immutable OCI image into the daemon-owned
/// cache. Images are deliberately not classified by execution phase: the
/// resulting digest-keyed cache is shared by build and guest consumers.
fn initialize_image_cache(context: &DevContext) -> Result<()> {
    build::build(context, &BuildSelection::runtime_only())?;
    let cache = context.image_cache();
    fs::create_dir_all(&cache)?;
    if fs::symlink_metadata(&cache)?.file_type().is_symlink() {
        return Err(DevError::Invalid(
            "OCI image cache must not be a symbolic link".into(),
        ));
    }
    let cache = fs::canonicalize(cache)?;
    let references = configured_oci_images()?;
    for reference in &references {
        let digest = image_digest(reference)?;
        let destination = cache.join(format!("sha256-{digest}"));
        if destination.join(".hephaestus-image").is_file() {
            let metadata = fs::symlink_metadata(&destination)?;
            let marker = destination.join(".hephaestus-image");
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || fs::symlink_metadata(&marker)?.file_type().is_symlink()
            {
                return Err(DevError::Invalid(format!(
                    "OCI image cache entry is unsafe: {}",
                    destination.display()
                )));
            }
            let recorded = fs::read_to_string(marker)?;
            if recorded.trim() != reference {
                return Err(DevError::Invalid(format!(
                    "OCI image cache entry has an unexpected immutable reference: {}",
                    destination.display()
                )));
            }
        } else {
            import_oci_image(context, reference, digest, &cache, &destination)?;
        }
    }
    build::install_guest_bootstrap(context)
        .and_then(|()| write_image_manifest(context, &references))
}

fn import_oci_image(
    context: &DevContext,
    reference: &str,
    digest: &str,
    cache: &Path,
    destination: &Path,
) -> Result<()> {
    let staging = cache.join(format!(".sha256-{digest}.{}", std::process::id()));
    remove_path(&staging)?;
    fs::create_dir(&staging)?;
    let container = context.image_container(digest);
    let _ignored = run_silent(Command::new("podman").args(["rm", "--force", &container]));
    let import_result = run(Command::new("podman").args(["pull", reference]))
        .and_then(|()| {
            run_silent(Command::new("podman").args([
                "create",
                "--name",
                &container,
                reference,
                "/bin/true",
            ]))
        })
        .and_then(|()| export_container(&container, &staging))
        .and_then(|()| configure_guest_identity(&staging))
        .and_then(|()| {
            fs::write(staging.join(".hephaestus-image"), reference)?;
            fs::rename(&staging, destination)?;
            Ok(())
        });
    let _ignored = run_silent(Command::new("podman").args(["rm", "--force", &container]));
    if import_result.is_err() {
        let _ignored = remove_path(&staging);
    }
    import_result
}

fn configured_oci_images() -> Result<Vec<String>> {
    let configured = env::var("HEPHAESTUS_LOCAL_OCI_IMAGES")
        .unwrap_or_else(|_| String::from(DEFAULT_LOCAL_OCI_IMAGE));
    let mut references = configured
        .split(',')
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if references.is_empty() || references.iter().any(String::is_empty) {
        return Err(DevError::Invalid(
            "HEPHAESTUS_LOCAL_OCI_IMAGES must be a non-empty comma-separated list".into(),
        ));
    }
    references.sort_unstable();
    references.dedup();
    for reference in &references {
        let _ignored = image_digest(reference)?;
    }
    Ok(references)
}

fn image_digest(reference: &str) -> Result<&str> {
    let Some((name, digest)) = reference.rsplit_once("@sha256:") else {
        return Err(DevError::Invalid(format!(
            "OCI image reference must be pinned with @sha256: {reference:?}"
        )));
    };
    let name_is_safe = !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'/' | b':' | b'_' | b'-')
        });
    if !name_is_safe
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DevError::Invalid(format!(
            "OCI image reference is not a safe immutable SHA-256 reference: {reference:?}"
        )));
    }
    Ok(digest)
}

#[derive(Serialize)]
struct ImageManifest {
    version: u32,
    roots: BTreeMap<String, ImageManifestEntry>,
}

#[derive(Serialize)]
struct ImageManifestEntry {
    kind: &'static str,
    path: PathBuf,
}

fn write_image_manifest(context: &DevContext, references: &[String]) -> Result<()> {
    let cache = fs::canonicalize(context.image_cache())?;
    let mut roots = BTreeMap::new();
    for reference in references {
        let digest = image_digest(reference)?;
        let path = fs::canonicalize(cache.join(format!("sha256-{digest}")))?;
        let marker = path.join(".hephaestus-image");
        if !path.starts_with(&cache)
            || fs::symlink_metadata(&path)?.file_type().is_symlink()
            || fs::symlink_metadata(&marker)?.file_type().is_symlink()
            || !marker.is_file()
        {
            return Err(DevError::Invalid(format!(
                "OCI image cache entry is unsafe: {}",
                path.display()
            )));
        }
        roots.insert(
            reference.clone(),
            ImageManifestEntry {
                kind: "directory",
                path,
            },
        );
    }
    let manifest = context.image_manifest();
    let temporary = manifest.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(
        &temporary,
        serde_json::to_vec(&ImageManifest { version: 1, roots }).map_err(|error| {
            DevError::Invalid(format!("cannot encode OCI image manifest: {error}"))
        })?,
    )?;
    fs::rename(temporary, manifest)?;
    Ok(())
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
