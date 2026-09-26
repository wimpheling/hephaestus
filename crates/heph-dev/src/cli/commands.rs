use super::{
    images::{CacheCommand, PlatformImageCommand, RepositoryImageCommand},
    logs::LogArgs,
    state::StateCommand,
};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "cargo dev",
    bin_name = "cargo dev",
    about = "Build and run the Hephaestus development environment",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Rebuild changed Rust components and restart them after successful builds.
    #[arg(long)]
    pub watch: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Build and run the complete stack in the foreground.
    Run(RunArgs),
    /// Build selected development components without starting the stack.
    Build(BuildSelection),
    /// Validate all host prerequisites.
    Doctor,
    /// Show service and state-resource health.
    Status,
    /// Read persisted component logs.
    Logs(LogArgs),
    /// Inspect, initialize, clean, or reinitialize development state.
    State {
        #[command(subcommand)]
        command: StateCommand,
    },
    /// Inspect or clean regenerable build caches.
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
    /// Explicitly build and install reviewed platform builder images.
    PlatformImages {
        #[command(subcommand)]
        command: PlatformImageCommand,
    },
    /// Explicitly enable or inspect repository-owned OCI image preparation.
    RepositoryImages {
        #[command(subcommand)]
        command: RepositoryImageCommand,
    },
    /// Run repository quality and architecture checks.
    Check {
        #[command(subcommand)]
        command: CheckCommand,
    },
    /// Run the complete repository quality gate (generated code, Rust, Phoenix, and UI).
    Quality,
    /// Run workspace Rust tests and generate HTML, LCOV, and per-crate coverage reports.
    Coverage(CoverageArgs),
}

#[derive(Debug, Args)]
pub struct CoverageArgs {
    /// Directory receiving Rust coverage reports, relative to the repository root.
    #[arg(long, default_value = "target/coverage")]
    pub output_dir: PathBuf,
}

#[derive(Clone, Copy, Debug, Subcommand)]
pub enum CheckCommand {
    /// Validate the architecture registry, exceptions, and enabled stable rules.
    Architecture,
    /// Check protobuf formatting, linting, and generated files when configured.
    Protobuf,
    /// Run Rust formatting, Clippy, tests, and documentation checks.
    Rust,
    /// Run Phoenix formatting, architecture, and tests.
    Phoenix,
    /// Run the UI architecture family and focused UI tests.
    Ui,
    /// Run every currently configured repository check.
    Full,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Rebuild changed Rust components and restart them after successful builds.
    #[arg(long)]
    pub watch: bool,
}

#[derive(Debug, Args)]
// Each Boolean deliberately corresponds to one independent CLI selector.
#[allow(clippy::struct_excessive_bools)]
pub struct BuildSelection {
    /// Build Phoenix, Elixir dependencies, JavaScript, and CSS.
    #[arg(long)]
    pub web: bool,
    /// Build the application daemon and supporting binaries.
    #[arg(long)]
    pub daemon: bool,
    /// Build the VM worker and guest bootstrap.
    #[arg(long)]
    pub runtime: bool,
    /// Build every component.
    #[arg(long)]
    pub all: bool,
}

impl BuildSelection {
    pub const fn web(&self) -> bool {
        self.all || self.none_selected() || self.web
    }

    pub const fn daemon(&self) -> bool {
        self.all || self.none_selected() || self.daemon
    }

    pub const fn runtime(&self) -> bool {
        self.all || self.none_selected() || self.runtime
    }

    const fn none_selected(&self) -> bool {
        !(self.web || self.daemon || self.runtime || self.all)
    }

    pub const fn rust_only() -> Self {
        Self {
            web: false,
            daemon: true,
            runtime: true,
            all: false,
        }
    }

    pub const fn daemon_only() -> Self {
        Self {
            web: false,
            daemon: true,
            runtime: false,
            all: false,
        }
    }

    pub const fn runtime_only() -> Self {
        Self {
            web: false,
            daemon: false,
            runtime: true,
            all: false,
        }
    }
}
