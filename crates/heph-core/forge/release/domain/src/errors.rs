/// Release-domain validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReleaseValueError {
    /// A stable key is malformed.
    #[error("{kind} must be a bounded lowercase identifier")]
    InvalidKey {
        /// Value-object kind, never rejected user input.
        kind: &'static str,
    },
    /// Release version is malformed.
    #[error("release version is invalid")]
    InvalidVersion,
    /// Artifact path is unsafe.
    #[error("artifact path must be relative, traversal-free, and outside .git")]
    InvalidArtifactPath,
    /// Ref selector is malformed.
    #[error("ref selector must be an exact fully-qualified ref or terminal prefix")]
    InvalidRefSelector,
    /// Project attempted to broaden release or platform policy.
    #[error("project runtime selection exceeds a release or platform ceiling")]
    PolicyBroadening,
    /// Candidate belongs to another source repository/family.
    #[error("candidate release agent belongs to a different agent family")]
    IncompatibleAgentFamily,
    /// Attachment repository belongs to another project.
    #[error("agent attachment repository belongs to a different project")]
    CrossProjectAttachment,
    /// UI key is malformed.
    #[error("UI key is invalid")]
    InvalidUiKey,
    /// UI route path is malformed.
    #[error("UI route path is invalid")]
    InvalidUiRoutePath,
    /// UI label is empty, oversized, or contains a control character.
    #[error("UI label is invalid")]
    InvalidUiLabel,
    /// UI repository Git access declaration is not one of the supported values.
    #[error("invalid UI repository Git access")]
    InvalidUiRepositoryGitAccess,
    /// UI declaration schema version is unsupported.
    #[error("UI schema version is unsupported")]
    UnsupportedUiSchemaVersion,
    /// UI media type is outside the explicit safe allowlist.
    #[error("UI media type is unsupported")]
    UnsupportedUiMediaType,
}
