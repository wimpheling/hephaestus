//! Host-private birth evidence before any owned allocation or formatting.
//!
//! A checked purpose is comparison data, not authorization or filesystem proof.
//! The later trusted caller must verify the original command and metadata seal.

mod codec;
pub mod filesystem;
mod journal;
mod namespace;
mod observation;
pub mod partial_retention;
mod phases;
mod pinned;
mod purpose;
mod readonly;
mod records;

use std::{fmt, io};

pub use journal::{FormatState, OwnedJournal, RecordedBacking};
pub use namespace::OwnedJournalLock;
pub use purpose::JournalPurpose;

/// A bounded journal failure; no unknown backing is changed on recovery failure.
#[derive(Debug)]
pub enum JournalError {
    /// Preserve bytes and require explicit recovery.
    RecoveryRequired(&'static str),
    /// Immutable purpose or path conflicts with an existing object.
    Conflict(&'static str),
    /// The existing per-volume OS lock is held by another process.
    Locked,
    /// Filesystem I/O failed before proof could be established.
    Io(io::Error),
}
impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecoveryRequired(reason) => {
                write!(f, "owned backing requires recovery: {reason}")
            }
            Self::Conflict(reason) => write!(f, "owned backing intent conflicts: {reason}"),
            Self::Locked => f.write_str("owned backing is exclusively locked"),
            Self::Io(error) => write!(f, "owned backing journal I/O failed: {error}"),
        }
    }
}
impl std::error::Error for JournalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
impl From<io::Error> for JournalError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<rustix::io::Errno> for JournalError {
    fn from(error: rustix::io::Errno) -> Self {
        Self::Io(error.into())
    }
}

pub const MAX_ATTEMPTS: u8 = 32;

#[cfg(test)]
mod tests;
