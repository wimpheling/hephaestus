use clap::{Args, Subcommand};

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
