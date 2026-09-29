//! `PostgreSQL` fixture database initialization.

use super::resources::create_volume;
use crate::{
    build,
    cli::BuildSelection,
    context::{DevContext, POSTGRES_IMAGE},
    process::{DevError, Result, run_quiet, run_silent},
};
use std::{fs, process::Command, thread, time::Duration};

pub(super) fn initialize_fixtures(context: &DevContext) -> Result<()> {
    fs::create_dir_all(context.local_root.join("repositories"))?;
    fs::create_dir_all(context.local_root.join("artifacts"))?;
    initialize_database(context, false)
}

pub(super) fn initialize_database(context: &DevContext, schema_only: bool) -> Result<()> {
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

pub(super) fn wait_for_postgres(context: &DevContext) -> Result<()> {
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

pub(super) fn run_seed(context: &DevContext, schema_only: bool) -> Result<()> {
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
