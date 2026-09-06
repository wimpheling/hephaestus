//! Typed development-environment CLI for the Hephaestus workspace.

mod build;
mod cache;
mod checks;
mod cli;
mod context;
mod diagnostics;
mod platform_images;
mod process;
mod repository_images;
mod state;
mod supervisor;
mod zot;

use clap::Parser;
use cli::{CacheCommand, Cli, Command, PlatformImageCommand, RepositoryImageCommand, StateCommand};
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
        Some(Command::PlatformImages { command }) => match command {
            PlatformImageCommand::Status => platform_images::status(&context),
            PlatformImageCommand::ImportBase => platform_images::import_base(&context),
            PlatformImageCommand::Build(arguments) => platform_images::build(&context, &arguments),
            PlatformImageCommand::Publish(arguments) => {
                platform_images::publish(&context, &arguments)
            }
            PlatformImageCommand::Clean(arguments) => platform_images::clean(&context, &arguments),
        },
        Some(Command::RepositoryImages { command }) => match command {
            RepositoryImageCommand::Status => repository_images::status(&context),
            RepositoryImageCommand::Enable(arguments) => {
                repository_images::enable(&context, &arguments)
            }
            RepositoryImageCommand::Disable => repository_images::disable(&context),
            RepositoryImageCommand::Clean => repository_images::clean(&context),
        },
        Some(Command::Check { command }) => checks::run(&context, command),
        Some(Command::Quality) => checks::quality(&context),
    }
}
