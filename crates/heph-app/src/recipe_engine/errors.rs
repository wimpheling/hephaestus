use capability_domain::CapabilitySlotKey;
use recipe_application::DeploymentError;
use release_service::InstanceExecutionError;
use thiserror::Error;

use crate::{
    recipe_instance_import::InstanceImportError, recipe_owned_volume::OwnedVolumeCreateError,
    recipe_retained_volume::RetainedVolumeVerificationError,
};

/// Held backend failures; none proves that an uncertain effect did not occur.
#[derive(Debug, Error)]
pub enum RecipeApplicationError {
    /// Fresh authorization, immutable input or ledger validation failed.
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    /// External verification requires a separately qualified adapter.
    #[error("external resource {0:?} requires a qualified verification adapter")]
    UnsupportedExternal(CapabilitySlotKey),
    /// An owning volume adapter retained the original operation or primary error.
    #[error("owned volume creation remains held: {0}")]
    Volume(Box<OwnedVolumeCreateError>),
    /// Retained qualification, pending custody and recovery failures remain explicit.
    #[error("retained volume verification remains held: {0}")]
    RetainedVolume(#[source] Box<RetainedVolumeVerificationError>),
    /// An exact import remains held; a new command cannot replace it.
    #[error("instance import remains held: {0}")]
    Instance(Box<InstanceImportError>),
    /// Creation may be ready while activation is denied, closed or unsupported.
    #[error("instance activation remains held: {0}")]
    Activation(#[from] InstanceExecutionError),
}
