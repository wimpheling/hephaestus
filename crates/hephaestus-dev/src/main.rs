//! Typed development-environment CLI for the Hephaestus workspace.

mod build;
mod cache;
mod checks;
mod cli;
mod context;
mod diagnostics;
mod process;
mod state;
mod supervisor;
mod zot;

use clap::Parser;
use cli::{CacheCommand, Cli, Command, StateCommand};
use context::DevContext;
use std::process::ExitCode;

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute() -> process::Result<()> {
    let cli = Cli::parse();
    let context = DevContext::discover()?;
    match cli.command {
        None => supervisor::run(&context, cli.watch),
        Some(Command::Run(arguments)) => supervisor::run(&context, cli.watch || arguments.watch),
        Some(Command::Build(selection)) => build::build(&context, &selection),
        Some(Command::Doctor) => diagnostics::doctor(&context),
        Some(Command::Status) => diagnostics::status(&context),
        Some(Command::Logs(arguments)) => diagnostics::logs(&context, &arguments),
        Some(Command::State { command }) => match command {
            StateCommand::List => state::list(&context),
            StateCommand::Init(selection) => state::init(&context, &selection),
            StateCommand::Clean(selection) => state::clean(&context, &selection),
            StateCommand::Reinit(selection) => state::reinit(&context, &selection),
        },
        Some(Command::Cache { command }) => match command {
            CacheCommand::List => {
                cache::list(&context);
                Ok(())
            }
            CacheCommand::Clean(selection) => cache::clean(&context, &selection),
        },
        Some(Command::Check { command }) => checks::run(&context, command),
        Some(Command::Quality) => checks::quality(&context),
    }
}
