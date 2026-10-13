//! Explicit query-shape selection; persisted Run profiles remain authoritative.

/// Scalar metadata SQL chosen by trusted application composition.
///
/// This selection creates no source or physical cleanup authority. Relation
/// presence, NULL owner labels and historical leases never choose it implicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarMetadataSchema {
    /// Retain the ordinary public107 columns and mutation semantics.
    Public107,
    /// Require explicit fresh Run profiles and exact qualified scalar helpers.
    Profiled,
}
