//! Provider-neutral contracts for immutable OCI images.

#[path = "image_domain/catalog.rs"]
mod catalog;
#[path = "image_domain/errors.rs"]
mod errors;
#[path = "image_domain/identifiers.rs"]
mod identifiers;
#[path = "image_domain/publication.rs"]
mod publication;

pub use catalog::{
    AvailabilityState, ImageProvenance, ImageRole, OciImage, ResolvedImage, Toolchain,
};
pub use errors::{ImageCatalogValueError, ImageSelectionError};
pub use identifiers::{ImageKey, OciDigest, OciImageId, OciImageReference};
pub use publication::{
    OciImagePublication, RegistryAvailabilityState, RegistryEvidence, RegistryEvidenceState,
    RegistryPublication, RegistryPublicationState,
};

#[cfg(test)]
#[path = "image_domain/tests.rs"]
mod tests;
