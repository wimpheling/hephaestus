use super::{CheckCommand, Cli, Command, StateCommand, StateResource};
use clap::Parser;
use std::path::PathBuf;

#[test]
fn no_state_selector_means_every_resource() {
    let cli = Cli::try_parse_from(["cargo-dev", "state", "clean"]).expect("valid state command");
    let Some(Command::State {
        command: StateCommand::Clean(selection),
    }) = cli.command
    else {
        panic!("expected state clean");
    };
    assert!(selection.selected(StateResource::Postgresql));
    assert!(selection.selected(StateResource::Zot));
    assert!(selection.selected(StateResource::Rootfs));
    assert!(selection.selected(StateResource::Logs));
}

#[test]
fn explicit_state_selectors_narrow_the_operation() {
    let cli = Cli::try_parse_from(["cargo-dev", "state", "reinit", "--postgresql", "--fixtures"])
        .expect("valid state command");
    let Some(Command::State {
        command: StateCommand::Reinit(selection),
    }) = cli.command
    else {
        panic!("expected state reinit");
    };
    assert!(selection.selected(StateResource::Postgresql));
    assert!(!selection.selected(StateResource::Zot));
    assert!(selection.selected(StateResource::Fixtures));
    assert!(!selection.selected(StateResource::Rootfs));
}

#[test]
fn watch_is_limited_to_run_commands() {
    assert!(Cli::try_parse_from(["cargo-dev", "--watch"]).is_ok());
    assert!(Cli::try_parse_from(["cargo-dev", "run", "--watch"]).is_ok());
    assert!(Cli::try_parse_from(["cargo-dev", "state", "list", "--watch"]).is_err());
}

#[test]
fn every_quality_check_has_an_independent_command() {
    for name in ["architecture", "protobuf", "rust", "phoenix", "ui", "full"] {
        let cli =
            Cli::try_parse_from(["cargo-dev", "check", name]).expect("valid quality check command");
        assert!(matches!(cli.command, Some(Command::Check { .. })));
    }

    let cli = Cli::try_parse_from(["cargo-dev", "check", "architecture"])
        .expect("valid architecture command");
    assert!(matches!(
        cli.command,
        Some(Command::Check {
            command: CheckCommand::Architecture
        })
    ));

    let cli =
        Cli::try_parse_from(["cargo-dev", "quality"]).expect("valid complete quality command");
    assert!(matches!(cli.command, Some(Command::Quality)));
}

#[test]
fn coverage_defaults_to_target_directory() {
    let cli = Cli::try_parse_from(["cargo-dev", "coverage"]).expect("valid coverage command");
    let Some(Command::Coverage(arguments)) = cli.command else {
        panic!("expected coverage command");
    };
    assert_eq!(arguments.output_dir, PathBuf::from("target/coverage"));
}

#[test]
fn coverage_accepts_an_explicit_output_directory() {
    let cli = Cli::try_parse_from([
        "cargo-dev",
        "coverage",
        "--output-dir",
        "/tmp/hephaestus-coverage",
    ])
    .expect("valid coverage output directory");
    let Some(Command::Coverage(arguments)) = cli.command else {
        panic!("expected coverage command");
    };
    assert_eq!(
        arguments.output_dir,
        PathBuf::from("/tmp/hephaestus-coverage")
    );
}
