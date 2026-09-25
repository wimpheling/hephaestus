use uuid::Uuid;

/// Immutable identifiers and hashes for the generated blog release artifact.
#[derive(Debug, Clone)]
pub struct PublishedCookingBlogArtifact {
    /// Build created for the approved or conflict-resolved source commit.
    pub build_id: Uuid,
    /// Published immutable release containing the generated site.
    pub release_id: Uuid,
    /// Artifact returned by the authorized Artifact service.
    pub artifact_id: Uuid,
    /// Exact Git object consumed by the build.
    pub source_commit: String,
    /// Hash returned by the Artifact service and checked against the database.
    pub sha256: String,
}
