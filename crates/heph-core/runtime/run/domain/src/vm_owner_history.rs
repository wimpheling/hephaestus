//! Immutable original VM owner comparison, never acquisition authority.

/// Why observed qualified originals disagree with the trusted configured owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmOwnerHistoryConflict {
    /// This configured host has an original qualified different VM namespace.
    NamespaceMismatch,
}

/// Why global original history cannot be attributed safely to a configured host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmOwnerHistoryUnknown {
    /// Pre-profile history cannot be assigned a qualified physical owner.
    HistoricalUnproven,
    /// An original has no complete, canonical owner identity.
    MissingOwnerIdentity,
    /// Protected original producer evidence is missing or contradictory.
    UnqualifiedOriginal,
}

/// One readonly snapshot of all original Run history, including closed terminals.
///
/// This comparison conveys no bootstrap, adoption, IO, recovery or physical
/// ownership authority. No-history is scoped to the supplied host, never global
/// database absence. Unattributable history from ANY host remains unknown rather
/// than disappearing behind a host predicate. Exact foreign-host originals may
/// be excluded only after their own immutable producer/owner qualification.
/// No result fences subsequent writes or proves that roots are physically empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmOwnerHistory {
    /// No qualified original belongs to the configured host and none is unknown.
    NoHistoryForConfiguredHost,
    /// Every original attributed to this host matches its configured namespace.
    Exact,
    /// At least one qualified original for this host has another namespace.
    Conflict(VmOwnerHistoryConflict),
    /// Complete global attribution could not be established.
    Unknown(VmOwnerHistoryUnknown),
}
