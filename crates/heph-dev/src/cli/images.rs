use clap::{Args, Subcommand};

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
    /// Import the one reviewed digest-pinned Ubuntu base into local Zot.
    ImportBase,
    /// Build the six reviewed platform images into a fresh private local release directory.
    Build(PlatformImageBuildArgs),
    /// Publish one reviewed local release, approve it, and provision its catalog.
    Publish(PlatformImagePublishArgs),
    /// Remove one completed local platform-image installation receipt.
    Clean(PlatformImageCleanArgs),
}

/// Explicit enablement and inspection for repository-owned OCI image builds.
#[derive(Debug, Subcommand)]
pub enum RepositoryImageCommand {
    /// Show whether the repository-image VM workflow is enabled and usable.
    Status,
    /// Enable the workflow from one immutable installed platform-image revision.
    Enable(RepositoryImageEnableArgs),
    /// Disable the workflow without deleting platform-image releases or catalog records.
    Disable,
    /// Remove only generated local repository-image workflow state.
    Clean,
}

#[derive(Debug, Args)]
pub struct RepositoryImageEnableArgs {
    /// Immutable platform-image revision previously published locally.
    #[arg(long)]
    pub revision: String,
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
