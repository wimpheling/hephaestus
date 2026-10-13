//! Explicit recorded publication origin, never historical actor credentials.
mod fresh;
mod original;
mod progress;
pub use fresh::QualifiedFreshInstanceClosurePublication;
pub use original::OriginalInstanceClosure;
pub use progress::InstanceClosurePublicationProgress;

/// Closed origin tag for immutable permanent closure publication.
///
/// Native142 qualifies original Create/config/admission/Pending/events before
/// selecting either branch. Pure construction is not provenance or current rights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceClosurePublicationOrigin {
    /// Strict historical publisher: closure actor equals original Drain actor.
    OriginalDrain,
    /// Actual recorded fresh publisher, separate from later cleanup's current user.
    QualifiedFreshRemove(Box<QualifiedFreshInstanceClosurePublication>),
}
