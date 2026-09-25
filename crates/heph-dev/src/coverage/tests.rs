use super::{
    CoveragePaths, IGNORE_FILENAME_REGEX, NATS_TEST_URL, POSTGRES_TEST_URL, coverage_command,
    lcov_command, report_command,
    services::{
        ServicePlan, ServiceUrls, apply_service_environment, parse_published_port,
        resolve_service_plan,
    },
    summary_command,
};
use std::{ffi::OsStr, path::Path, process::Command};

#[test]
fn relative_output_is_rooted_and_has_stable_artifact_names() {
    let paths = CoveragePaths::new(
        Path::new("/workspace/hephaestus"),
        Path::new("target/coverage"),
    );
    assert_eq!(
        paths.output,
        Path::new("/workspace/hephaestus/target/coverage")
    );
    assert_eq!(
        paths.html,
        Path::new("/workspace/hephaestus/target/coverage/html")
    );
    assert_eq!(
        paths.html.join("index.html"),
        Path::new("/workspace/hephaestus/target/coverage/html/index.html")
    );
    assert_eq!(
        paths.summary,
        Path::new("/workspace/hephaestus/target/coverage/summary.txt")
    );
    assert_eq!(
        paths.lcov,
        Path::new("/workspace/hephaestus/target/coverage/lcov.info")
    );
    assert_eq!(
        paths.csv,
        Path::new("/workspace/hephaestus/target/coverage/rust-coverage-by-crate.csv")
    );
    assert_eq!(
        paths.markdown,
        Path::new("/workspace/hephaestus/target/coverage/rust-coverage-by-crate.md")
    );
}

#[test]
fn absolute_output_is_preserved() {
    let paths = CoveragePaths::new(Path::new("/workspace/hephaestus"), Path::new("/tmp/cov"));
    assert_eq!(paths.output, Path::new("/tmp/cov"));
}

#[test]
fn cargo_commands_match_ci_coverage_flags() {
    let root = Path::new("/workspace/hephaestus");
    let output = Path::new("/workspace/hephaestus/target/coverage");
    let lcov = Path::new("/workspace/hephaestus/target/coverage/lcov.info");
    let coverage = coverage_command(root, output);
    assert_eq!(coverage.get_program(), OsStr::new("cargo"));
    assert_eq!(
        coverage.get_args().collect::<Vec<_>>(),
        vec![
            OsStr::new("llvm-cov"),
            OsStr::new("--workspace"),
            OsStr::new("--all-features"),
            OsStr::new("--ignore-filename-regex"),
            OsStr::new(IGNORE_FILENAME_REGEX),
            OsStr::new("--html"),
            OsStr::new("--output-dir"),
            output.as_os_str(),
            OsStr::new("--"),
            OsStr::new("--test-threads=1"),
        ]
    );
    let report = report_command(root);
    assert_eq!(
        report.get_args().collect::<Vec<_>>(),
        vec![
            OsStr::new("llvm-cov"),
            OsStr::new("report"),
            OsStr::new("--ignore-filename-regex"),
            OsStr::new(IGNORE_FILENAME_REGEX),
        ]
    );
    let lcov_command = lcov_command(root, lcov);
    assert_eq!(
        lcov_command.get_args().collect::<Vec<_>>(),
        vec![
            OsStr::new("llvm-cov"),
            OsStr::new("report"),
            OsStr::new("--ignore-filename-regex"),
            OsStr::new(IGNORE_FILENAME_REGEX),
            OsStr::new("--lcov"),
            OsStr::new("--output-path"),
            lcov.as_os_str(),
        ]
    );
}

#[test]
fn summary_command_passes_all_report_paths() {
    let root = Path::new("/workspace/hephaestus");
    let paths = CoveragePaths::new(root, Path::new("target/coverage"));
    let command = summary_command(root, &root.join("scripts/rust_coverage_summary.py"), &paths);
    assert_eq!(command.get_program(), OsStr::new("python3"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![
            root.join("scripts/rust_coverage_summary.py").as_os_str(),
            OsStr::new("--repo-root"),
            root.as_os_str(),
            OsStr::new("--lcov"),
            paths.lcov.as_os_str(),
            OsStr::new("--csv"),
            paths.csv.as_os_str(),
            OsStr::new("--markdown"),
            paths.markdown.as_os_str(),
        ]
    );
}

#[test]
fn service_plan_inherits_only_a_complete_external_pair() {
    assert_eq!(
        resolve_service_plan(Some("postgres://external"), Some("nats://external"))
            .expect("complete external pair"),
        ServicePlan::External(ServiceUrls {
            postgres: "postgres://external".into(),
            nats: "nats://external".into(),
        })
    );
    assert_eq!(
        resolve_service_plan(None, None).expect("managed service plan"),
        ServicePlan::Managed
    );
    assert!(resolve_service_plan(Some("postgres://external"), None).is_err());
    assert!(resolve_service_plan(None, Some("nats://external")).is_err());
}

#[test]
fn published_port_requires_loopback_and_a_numeric_port() {
    assert_eq!(
        parse_published_port("127.0.0.1:45432").expect("valid loopback mapping"),
        45432
    );
    assert!(parse_published_port("0.0.0.0:45432").is_err());
    assert!(parse_published_port("127.0.0.1:not-a-port").is_err());
    assert!(parse_published_port("missing-port").is_err());
}

#[test]
fn managed_service_urls_are_applied_to_test_commands() {
    let urls = ServiceUrls {
        postgres: "postgres://postgres:postgres@127.0.0.1:45432/hephaestus".into(),
        nats: "nats://127.0.0.1:45433".into(),
    };
    let mut command = Command::new("cargo");
    apply_service_environment(&mut command, Some(&urls));
    assert_eq!(
        command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(POSTGRES_TEST_URL))
            .and_then(|(_, value)| value),
        Some(OsStr::new(&urls.postgres))
    );
    assert_eq!(
        command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(NATS_TEST_URL))
            .and_then(|(_, value)| value),
        Some(OsStr::new(&urls.nats))
    );
}
