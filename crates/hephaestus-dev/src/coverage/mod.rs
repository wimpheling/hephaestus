mod services;

#[cfg(test)]
mod tests;

use crate::{
    cli::CoverageArgs,
    context::DevContext,
    process::{DevError, Result, command_exists, run as run_process},
};
use services::{apply_service_environment, coverage_services};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
};

const IGNORE_FILENAME_REGEX: &str = "crates/rpc-proto/src/generated/";
const SUMMARY_SCRIPT: &str = "scripts/rust_coverage_summary.py";
const POSTGRES_TEST_URL: &str = "HEPHAESTUS_POSTGRES_TEST_URL";
const NATS_TEST_URL: &str = "HEPHAESTUS_NATS_TEST_URL";
const CONTAINER_ENGINE: &str = "CONTAINER_ENGINE";
const POSTGRES_IMAGE: &str = "docker.io/library/postgres@sha256:af194ccf3e2d7fe367012c7b88ce8b816c5c889b18a5b316799a1f0d7eac746a";
const NATS_IMAGE: &str = "docker.io/library/nats@sha256:e4bf19f15fd3218814a4e3c9e0064e1334bd8aa20d5984b9f1a0afd084f8cc00";
const SERVICE_WAIT_ATTEMPTS: usize = 300;
const SERVICE_WAIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

#[derive(Debug, Eq, PartialEq)]
struct CoveragePaths {
    output: PathBuf,
    html: PathBuf,
    summary: PathBuf,
    lcov: PathBuf,
    csv: PathBuf,
    markdown: PathBuf,
}

impl CoveragePaths {
    fn new(repository_root: &Path, requested_output: &Path) -> Self {
        let output = if requested_output.is_absolute() {
            requested_output.to_owned()
        } else {
            repository_root.join(requested_output)
        };
        Self {
            html: output.join("html"),
            summary: output.join("summary.txt"),
            lcov: output.join("lcov.info"),
            csv: output.join("rust-coverage-by-crate.csv"),
            markdown: output.join("rust-coverage-by-crate.md"),
            output,
        }
    }
}

pub fn run(context: &DevContext, arguments: &CoverageArgs) -> Result<()> {
    let repository_root = &context.repository_root;
    let paths = CoveragePaths::new(repository_root, &arguments.output_dir);
    require_tool(
        "cargo-llvm-cov",
        "install cargo-llvm-cov with cargo install cargo-llvm-cov",
    )?;
    require_tool(
        "python3",
        "install Python 3.11 or newer before running Rust coverage",
    )?;
    require_python_tomllib()?;
    let summary_script = repository_root.join(SUMMARY_SCRIPT);
    if !summary_script.is_file() {
        return Err(DevError::Invalid(format!(
            "Rust coverage summary script is missing at {}",
            summary_script.display()
        )));
    }

    fs::create_dir_all(&paths.html)?;
    let services = coverage_services()?;
    if services.is_some() {
        println!("Starting disposable PostgreSQL and NATS coverage services");
    }
    println!("Rust coverage output: {}", paths.output.display());
    let urls = services.as_ref().map(|service| &service.urls);
    let mut coverage = coverage_command(repository_root, &paths.output);
    apply_service_environment(&mut coverage, urls);
    run_process(&mut coverage)?;
    run_report(repository_root, &paths.summary)?;
    run_process(&mut lcov_command(repository_root, &paths.lcov))?;
    run_process(&mut summary_command(
        repository_root,
        &summary_script,
        &paths,
    ))?;

    let html_index = paths.html.join("index.html");
    println!("Rust coverage artifacts:");
    for path in [
        html_index.as_path(),
        paths.summary.as_path(),
        paths.lcov.as_path(),
        paths.csv.as_path(),
        paths.markdown.as_path(),
    ] {
        println!("  {}", path.display());
    }
    Ok(())
}

fn require_tool(program: &str, hint: &str) -> Result<()> {
    if command_exists(program) {
        Ok(())
    } else {
        Err(DevError::Invalid(format!(
            "Rust coverage requires {program} in PATH; {hint}"
        )))
    }
}

fn require_python_tomllib() -> Result<()> {
    let status = Command::new("python3")
        .args(["-c", "import tomllib"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(DevError::Invalid(
            "Rust coverage requires Python 3.11 or newer with the tomllib module".into(),
        ))
    }
}

fn coverage_command(repository_root: &Path, html_directory: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .args([
            "llvm-cov",
            "--workspace",
            "--all-features",
            "--ignore-filename-regex",
            IGNORE_FILENAME_REGEX,
            "--html",
            "--output-dir",
        ])
        .arg(html_directory)
        .args(["--", "--test-threads=1"])
        .current_dir(repository_root);
    command
}

fn report_command(repository_root: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .args([
            "llvm-cov",
            "report",
            "--ignore-filename-regex",
            IGNORE_FILENAME_REGEX,
        ])
        .current_dir(repository_root);
    command
}

fn lcov_command(repository_root: &Path, lcov_path: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .args([
            "llvm-cov",
            "report",
            "--ignore-filename-regex",
            IGNORE_FILENAME_REGEX,
            "--lcov",
            "--output-path",
        ])
        .arg(lcov_path)
        .current_dir(repository_root);
    command
}

fn summary_command(repository_root: &Path, script: &Path, paths: &CoveragePaths) -> Command {
    let mut command = Command::new("python3");
    command
        .arg(script)
        .args(["--repo-root"])
        .arg(repository_root)
        .args(["--lcov"])
        .arg(&paths.lcov)
        .args(["--csv"])
        .arg(&paths.csv)
        .args(["--markdown"])
        .arg(&paths.markdown)
        .current_dir(repository_root);
    command
}

fn run_report(repository_root: &Path, summary_path: &Path) -> Result<()> {
    let mut command = report_command(repository_root);
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command.output()?;
    write_report_output(&output, summary_path)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(DevError::Command {
            program,
            status: output.status,
        })
    }
}

fn write_report_output(output: &Output, summary_path: &Path) -> Result<()> {
    let mut summary = File::create(summary_path)?;
    summary.write_all(&output.stdout)?;
    summary.write_all(&output.stderr)?;
    io::stdout().write_all(&output.stdout)?;
    io::stderr().write_all(&output.stderr)?;
    Ok(())
}
