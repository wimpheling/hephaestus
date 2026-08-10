use clap::{Args, Parser, Subcommand, ValueEnum};

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
    /// Run repository quality and architecture checks.
    Check {
        #[command(subcommand)]
        command: CheckCommand,
    },
    /// Run the complete repository quality gate (generated code, Rust, Phoenix, and UI).
    Quality,
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

#[derive(Debug, Subcommand)]
pub enum StateCommand {
    /// Show location, existence, and size of every state resource.
    List,
    /// Create selected missing resources idempotently.
    Init(StateSelection),
    /// Delete selected resources.
    Clean(StateSelection),
    /// Clean and then initialize selected resources.
    Reinit(StateSelection),
}

#[derive(Clone, Debug, Args)]
// State resources must remain independently selectable from the command line.
#[allow(clippy::struct_excessive_bools)]
pub struct StateSelection {
    /// Postgres database volume, roles, schema, and migrations.
    #[arg(long)]
    pub postgresql: bool,
    /// NATS durable stream volume.
    #[arg(long)]
    pub nats: bool,
    /// Forge-owned Zot OCI registry storage, configuration, and verifier.
    #[arg(long)]
    pub zot: bool,
    /// Bare Git repositories.
    #[arg(long)]
    pub repositories: bool,
    /// Immutable release artifacts.
    #[arg(long)]
    pub artifacts: bool,
    /// Persistent agent state volumes.
    #[arg(long)]
    pub agent_volumes: bool,
    /// Active, build, sealed, and result workspaces.
    #[arg(long)]
    pub workspaces: bool,
    /// Local secret-wrapping keyring.
    #[arg(long)]
    pub secret_keys: bool,
    /// Configured immutable OCI images and their guest bootstrap.
    #[arg(long)]
    pub rootfs: bool,
    /// Seeded identity, organization, repository, release, and metadata fixture.
    #[arg(long)]
    pub fixtures: bool,
    /// VM sockets, ephemeral mounts, raw-secret storage, and control metadata.
    #[arg(long)]
    pub runtime: bool,
    /// Web, daemon, and OIDC logs.
    #[arg(long)]
    pub logs: bool,
    /// Every state resource.
    #[arg(long)]
    pub all: bool,
}

impl StateSelection {
    pub const fn selected(&self, resource: StateResource) -> bool {
        self.all
            || self.none_selected()
            || match resource {
                StateResource::Postgresql => self.postgresql,
                StateResource::Nats => self.nats,
                StateResource::Zot => self.zot,
                StateResource::Repositories => self.repositories,
                StateResource::Artifacts => self.artifacts,
                StateResource::AgentVolumes => self.agent_volumes,
                StateResource::Workspaces => self.workspaces,
                StateResource::SecretKeys => self.secret_keys,
                StateResource::Rootfs => self.rootfs,
                StateResource::Fixtures => self.fixtures,
                StateResource::Runtime => self.runtime,
                StateResource::Logs => self.logs,
            }
    }

    const fn none_selected(&self) -> bool {
        !(self.postgresql
            || self.nats
            || self.zot
            || self.repositories
            || self.artifacts
            || self.agent_volumes
            || self.workspaces
            || self.secret_keys
            || self.rootfs
            || self.fixtures
            || self.runtime
            || self.logs
            || self.all)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum StateResource {
    Postgresql,
    Nats,
    Zot,
    Repositories,
    Artifacts,
    AgentVolumes,
    Workspaces,
    SecretKeys,
    Rootfs,
    Fixtures,
    Runtime,
    Logs,
}

pub const STATE_RESOURCES: [StateResource; 12] = [
    StateResource::Postgresql,
    StateResource::Nats,
    StateResource::Zot,
    StateResource::Repositories,
    StateResource::Artifacts,
    StateResource::AgentVolumes,
    StateResource::Workspaces,
    StateResource::SecretKeys,
    StateResource::Rootfs,
    StateResource::Fixtures,
    StateResource::Runtime,
    StateResource::Logs,
];

#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Show the location and size of every regenerable cache.
    List,
    /// Delete selected regenerable caches.
    Clean(CacheSelection),
}

#[derive(Debug, Subcommand)]
pub enum PlatformImageCommand {
    /// Show persisted local platform image release and installation state.
    Status,
    /// Build the four reviewed platform images into a fresh private local release directory.
    Build(PlatformImageBuildArgs),
    /// Publish one reviewed local release, approve it, and provision its catalog.
    Publish(PlatformImagePublishArgs),
    /// Remove one completed local platform-image installation receipt.
    Clean(PlatformImageCleanArgs),
}

#[derive(Debug, Args)]
pub struct PlatformImageBuildArgs {
    /// Immutable source URI recorded in the release provenance.
    #[arg(long)]
    pub source: String,
    /// Exact lowercase source commit (40 or 64 hexadecimal characters).
    #[arg(long)]
    pub revision: String,
    /// Exact UTC RFC3339 creation time recorded in the release provenance.
    #[arg(long)]
    pub created: String,
}

#[derive(Debug, Args)]
pub struct PlatformImagePublishArgs {
    /// Immutable release revision previously created by `platform-images build`.
    #[arg(long)]
    pub revision: String,
}

#[derive(Debug, Args)]
pub struct PlatformImageCleanArgs {
    /// Immutable release revision whose private installation receipt is removed.
    #[arg(long)]
    pub revision: String,
}

#[derive(Debug, Args)]
// Regenerable caches must remain independently selectable.
#[allow(clippy::struct_excessive_bools)]
pub struct CacheSelection {
    /// Cargo build output.
    #[arg(long)]
    pub rust: bool,
    /// Mix build output and downloaded Elixir dependencies.
    #[arg(long)]
    pub elixir: bool,
    /// Node dependencies used by browser tests and web assets.
    #[arg(long)]
    pub node: bool,
    /// Project-pinned Podman images.
    #[arg(long)]
    pub containers: bool,
    /// Every regenerable cache.
    #[arg(long)]
    pub all: bool,
}

impl CacheSelection {
    pub const fn rust(&self) -> bool {
        self.all || self.none_selected() || self.rust
    }

    pub const fn elixir(&self) -> bool {
        self.all || self.none_selected() || self.elixir
    }

    pub const fn node(&self) -> bool {
        self.all || self.none_selected() || self.node
    }

    pub const fn containers(&self) -> bool {
        self.all || self.none_selected() || self.containers
    }

    const fn none_selected(&self) -> bool {
        !(self.rust || self.elixir || self.node || self.containers || self.all)
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum LogComponent {
    Web,
    Daemon,
    Oidc,
    Zot,
}

#[derive(Debug, Args)]
pub struct LogArgs {
    /// Component to read; omit to show the end of every component log.
    #[arg(value_enum)]
    pub component: Option<LogComponent>,
    /// Continue following the selected log.
    #[arg(short, long, requires = "component")]
    pub follow: bool,
}

#[cfg(test)]
mod tests {
    use super::{CheckCommand, Cli, Command, StateCommand, StateResource};
    use clap::Parser;

    #[test]
    fn no_state_selector_means_every_resource() {
        let cli =
            Cli::try_parse_from(["cargo-dev", "state", "clean"]).expect("valid state command");
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
        let cli =
            Cli::try_parse_from(["cargo-dev", "state", "reinit", "--postgresql", "--fixtures"])
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
            let cli = Cli::try_parse_from(["cargo-dev", "check", name])
                .expect("valid quality check command");
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
}
