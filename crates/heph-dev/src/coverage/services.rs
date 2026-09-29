use super::{
    CONTAINER_ENGINE, NATS_IMAGE, NATS_TEST_URL, POSTGRES_IMAGE, POSTGRES_TEST_URL,
    SERVICE_WAIT_ATTEMPTS, SERVICE_WAIT_INTERVAL,
};
use crate::process::{DevError, Result, command_exists, run as run_process};
use std::{
    env,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ServiceUrls {
    pub(super) postgres: String,
    pub(super) nats: String,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum ServicePlan {
    External(ServiceUrls),
    Managed,
}

pub(super) struct CoverageServices {
    engine: String,
    postgres_container: String,
    nats_container: String,
    postgres_started: bool,
    nats_started: bool,
    pub(super) urls: ServiceUrls,
    stop_requested: Arc<AtomicBool>,
}

pub(super) fn coverage_services() -> Result<Option<CoverageServices>> {
    let postgres = env::var(POSTGRES_TEST_URL)
        .ok()
        .filter(|value| !value.is_empty());
    let nats = env::var(NATS_TEST_URL)
        .ok()
        .filter(|value| !value.is_empty());
    match resolve_service_plan(postgres.as_deref(), nats.as_deref())? {
        ServicePlan::External(_) => Ok(None),
        ServicePlan::Managed => CoverageServices::start().map(Some),
    }
}

pub(super) fn resolve_service_plan(
    postgres: Option<&str>,
    nats: Option<&str>,
) -> Result<ServicePlan> {
    match (postgres, nats) {
        (Some(postgres), Some(nats)) => Ok(ServicePlan::External(ServiceUrls {
            postgres: postgres.to_owned(),
            nats: nats.to_owned(),
        })),
        (None, None) => Ok(ServicePlan::Managed),
        _ => Err(DevError::Invalid(format!(
            "Rust coverage requires both {POSTGRES_TEST_URL} and {NATS_TEST_URL}, or neither; set both URLs or unset both to start disposable services"
        ))),
    }
}

impl CoverageServices {
    fn start() -> Result<Self> {
        let engine = select_engine()?;
        let stop_requested = Arc::new(AtomicBool::new(false));
        let handler_flag = Arc::clone(&stop_requested);
        ctrlc::set_handler(move || {
            handler_flag.store(true, Ordering::SeqCst);
        })
        .map_err(|error| {
            DevError::Invalid(format!(
                "could not install coverage Ctrl-C handler: {error}"
            ))
        })?;

        let mut services = Self {
            engine,
            postgres_container: unique_container_name("postgres"),
            nats_container: unique_container_name("nats"),
            postgres_started: false,
            nats_started: false,
            urls: ServiceUrls {
                postgres: String::new(),
                nats: String::new(),
            },
            stop_requested,
        };
        services.start_container("postgres")?;
        let postgres_port = published_port(&services.engine, &services.postgres_container, 5432)?;
        services.start_container("nats")?;
        let nats_port = published_port(&services.engine, &services.nats_container, 4222)?;
        services.urls = ServiceUrls {
            postgres: format!(
                "postgres://postgres:postgres@127.0.0.1:{postgres_port}/hephaestus?sslmode=disable"
            ),
            nats: format!("nats://127.0.0.1:{nats_port}"),
        };
        services.wait_for_postgres()?;
        services.wait_for_nats()?;
        Ok(services)
    }

    fn start_container(&mut self, service: &str) -> Result<()> {
        let (container, container_port, image) = match service {
            "postgres" => (&self.postgres_container, 5432, POSTGRES_IMAGE),
            "nats" => (&self.nats_container, 4222, NATS_IMAGE),
            _ => {
                return Err(DevError::Invalid(format!(
                    "unknown coverage service: {service}"
                )));
            }
        };
        if container_exists(&self.engine, container)? {
            return Err(DevError::Invalid(format!(
                "coverage container name is already in use: {container}"
            )));
        }
        let publish = format!("127.0.0.1::{container_port}");
        let mut command = Command::new(&self.engine);
        command
            .args(["run", "--detach", "--rm", "--name"])
            .arg(container)
            .args(["--publish"])
            .arg(publish);
        if service == "postgres" {
            command.args([
                "--env",
                "POSTGRES_PASSWORD=postgres",
                "--env",
                "POSTGRES_DB=hephaestus",
            ]);
            command.arg(image);
        } else {
            command.arg(image);
            command.arg("-js");
        }
        command.stdout(Stdio::null());
        run_process(&mut command)?;
        if service == "postgres" {
            self.postgres_started = true;
        } else {
            self.nats_started = true;
        }
        Ok(())
    }

    fn wait_for_postgres(&self) -> Result<()> {
        for _ in 0..SERVICE_WAIT_ATTEMPTS {
            if self.stop_requested.load(Ordering::SeqCst) {
                return Err(DevError::Invalid(
                    "Rust coverage interrupted while waiting for PostgreSQL".into(),
                ));
            }
            let ready = service_status(
                &self.engine,
                &[
                    "exec",
                    &self.postgres_container,
                    "pg_isready",
                    "--quiet",
                    "--username",
                    "postgres",
                    "--dbname",
                    "hephaestus",
                ],
            )?;
            if ready {
                return Ok(());
            }
            thread::sleep(SERVICE_WAIT_INTERVAL);
        }
        Err(DevError::Invalid(format!(
            "timed out waiting for coverage PostgreSQL container {}",
            self.postgres_container
        )))
    }

    fn wait_for_nats(&self) -> Result<()> {
        for _ in 0..SERVICE_WAIT_ATTEMPTS {
            if self.stop_requested.load(Ordering::SeqCst) {
                return Err(DevError::Invalid(
                    "Rust coverage interrupted while waiting for NATS".into(),
                ));
            }
            let output = Command::new(&self.engine)
                .args(["logs", &self.nats_container])
                .output()?;
            let logs = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if output.status.success() && logs.contains("Server is ready") {
                return Ok(());
            }
            thread::sleep(SERVICE_WAIT_INTERVAL);
        }
        Err(DevError::Invalid(format!(
            "timed out waiting for coverage NATS container {}",
            self.nats_container
        )))
    }
}

impl Drop for CoverageServices {
    fn drop(&mut self) {
        for (container, started) in [
            (&self.nats_container, self.nats_started),
            (&self.postgres_container, self.postgres_started),
        ] {
            if started {
                let _ = Command::new(&self.engine)
                    .args(["rm", "--force"])
                    .arg(container)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
}

fn select_engine() -> Result<String> {
    if let Some(configured) = env::var_os(CONTAINER_ENGINE) {
        let configured = configured.to_string_lossy().into_owned();
        if configured != "podman" && configured != "docker" {
            return Err(DevError::Invalid(format!(
                "{CONTAINER_ENGINE} must be podman or docker, got {configured:?}"
            )));
        }
        if !command_exists(&configured) {
            return Err(DevError::Invalid(format!(
                "configured coverage container engine is unavailable: {configured}"
            )));
        }
        return Ok(configured);
    }
    for candidate in ["podman", "docker"] {
        if command_exists(candidate) {
            return Ok(candidate.to_owned());
        }
    }
    Err(DevError::Invalid(
        "Rust coverage needs Podman or Docker to start disposable PostgreSQL and NATS services; install one or set CONTAINER_ENGINE"
            .into(),
    ))
}

fn unique_container_name(service: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!(
        "hephaestus-coverage-{service}-{}-{timestamp}",
        std::process::id()
    )
}

fn container_exists(engine: &str, container: &str) -> Result<bool> {
    Ok(Command::new(engine)
        .args(["inspect", container])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success())
}

fn service_status(engine: &str, arguments: &[&str]) -> Result<bool> {
    Ok(Command::new(engine)
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success())
}

pub(super) fn parse_published_port(mapping: &str) -> Result<u16> {
    let mapping = mapping.trim();
    let Some((host, port)) = mapping.rsplit_once(':') else {
        return Err(DevError::Invalid(format!(
            "container engine returned malformed loopback port mapping: {mapping:?}"
        )));
    };
    if host != "127.0.0.1" {
        return Err(DevError::Invalid(format!(
            "container engine exposed coverage service outside loopback: {mapping:?}"
        )));
    }
    port.parse::<u16>().map_err(|_| {
        DevError::Invalid(format!(
            "container engine returned an invalid host port: {mapping:?}"
        ))
    })
}

fn published_port(engine: &str, container: &str, container_port: u16) -> Result<u16> {
    let mapping = Command::new(engine)
        .args(["port", container])
        .arg(format!("{container_port}/tcp"))
        .output()?;
    if !mapping.status.success() {
        return Err(DevError::Command {
            program: engine.into(),
            status: mapping.status,
        });
    }
    let mapping_text = String::from_utf8_lossy(&mapping.stdout);
    let line = mapping_text
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| DevError::Invalid(format!("{engine} returned no port for {container}")))?;
    parse_published_port(line)
}

pub(super) fn apply_service_environment(command: &mut Command, urls: Option<&ServiceUrls>) {
    if let Some(urls) = urls {
        command.env(POSTGRES_TEST_URL, &urls.postgres);
        command.env(NATS_TEST_URL, &urls.nats);
    }
}
