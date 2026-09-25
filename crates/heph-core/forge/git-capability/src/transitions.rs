/// A trusted ref transition determined from repository state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefTransition {
    /// Create a previously absent ref.
    Create,
    /// Change an existing ref, with ancestry already verified.
    Update {
        /// Whether the old object is an ancestor of the new object.
        fast_forward: bool,
    },
    /// Delete an existing ref.
    Delete,
}

/// One trusted changed-path record from a proposed receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathChange<'a> {
    /// A new path.
    Addition(&'a str),
    /// An existing path with changed content or mode.
    Modification(&'a str),
    /// A removed path.
    Deletion(&'a str),
    /// A rename or copy whose source and destination both require authority.
    Rename {
        /// Original repository-relative path.
        from: &'a str,
        /// New repository-relative path.
        to: &'a str,
    },
}

impl<'a> PathChange<'a> {
    const fn paths(self) -> [Option<&'a str>; 2] {
        match self {
            Self::Addition(path) | Self::Modification(path) | Self::Deletion(path) => {
                [Some(path), None]
            }
            Self::Rename { from, to } => [Some(from), Some(to)],
        }
    }
}

/// Trusted input for checking one atomic receive ref command.
#[derive(Debug, Clone, Copy)]
pub struct ReceiveUpdate<'a> {
    /// Fully qualified proposed ref.
    pub reference: &'a str,
    /// Creation, verified update, or deletion.
    pub transition: RefTransition,
    /// Complete path delta required by [`crate::GitCapabilityScope::allows_receive`].
    pub changed_paths: &'a [PathChange<'a>],
}

pub const fn paths(change: PathChange<'_>) -> [Option<&str>; 2] {
    change.paths()
}
