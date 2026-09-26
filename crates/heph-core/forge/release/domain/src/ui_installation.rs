//! Domain primitives for project and repository UI installations.

mod digest;
mod identity;

pub use digest::UiInstallationInputDigest;
pub use identity::{
    MAX_UI_INSTALLATION_CALLER_KEY_BYTES, UiInstallationCallerKey, UiInstallationCommandIdentity,
    UiInstallationOperation, UiInstallationState, UiInstallationTarget,
};

#[cfg(test)]
mod tests;
