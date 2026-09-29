//! Typed development-environment CLI declarations.

mod commands;
mod images;
mod logs;
mod state;

#[cfg(test)]
mod tests;

pub use commands::{BuildSelection, CheckCommand, Cli, Command, CoverageArgs};
// Preserve the original public module path for command argument consumers.
#[allow(unused_imports)]
pub use commands::RunArgs;
pub use images::{
    CacheCommand, CacheSelection, PlatformImageBuildArgs, PlatformImageCleanArgs,
    PlatformImageCommand, PlatformImagePublishArgs, RepositoryImageCommand,
    RepositoryImageEnableArgs,
};
pub use logs::{LogArgs, LogComponent};
pub use state::{STATE_RESOURCES, StateCommand, StateResource, StateSelection};
