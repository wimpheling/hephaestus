use registry_domain::{RegistryValueError, SupplyChainReferrerKind};

/// Opaque failure from a bounded OCI read operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RegistryReadError {
    /// The scoped bearer token was rejected by the registry.
    #[error("registry authentication failed")]
    AuthenticationFailed,
    /// The registry could not provide a valid bounded response.
    #[error("registry read failed")]
    Failed,
}

/// Non-sensitive command launch failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandRunnerError {
    /// The configured executable could not be launched.
    #[error("OCI client executable could not be launched")]
    LaunchFailed,
}

/// Controlled publisher failure. No variant contains a bearer token or command output.
#[derive(Debug, thiserror::Error)]
pub enum PublisherError {
    /// The durable intent points at another registry authority.
    #[error("publication intent authority differs from publisher configuration")]
    AuthorityMismatch,
    /// The durable intent was already verified or approved.
    #[error("publication intent has already established immutable verification")]
    AlreadyVerified,
    /// The durable intent cannot be retried.
    #[error("publication intent is retired or missing")]
    NonRetryableIntent,
    /// A path was outside the configured root, relative, or contained a symlink.
    #[error("publisher input path is unsafe")]
    UnsafePath,
    /// A filesystem operation failed without exposing a sensitive path or payload.
    #[error("publisher filesystem operation failed")]
    Filesystem(#[source] std::io::Error),
    /// The local OCI layout root file was malformed.
    #[error("local OCI layout metadata is malformed")]
    MalformedLocalLayout,
    /// The local OCI index was malformed.
    #[error("local OCI index is malformed")]
    MalformedLocalIndex,
    /// The local index did not contain exactly the intended digest.
    #[error("local OCI layout does not contain the intended digest")]
    WrongLocalDigest,
    /// The local OCI layout holds more than one possible publication subject.
    #[error("local OCI layout has an ambiguous publication subject")]
    AmbiguousLocalLayout,
    /// The local OCI layout lacks the administrator-required immutable source tag.
    #[error("local OCI layout does not bind its source tag to the intent digest")]
    WrongLocalReferenceName,
    /// The local descriptor did not exactly match the durable publication intent.
    #[error("local OCI descriptor differs from publication intent")]
    WrongLocalDescriptor,
    /// A signature is required by policy but no trusted signature file was supplied.
    #[error("required signature evidence is absent")]
    MissingSignature,
    /// The OCI client could not be launched.
    #[error("OCI client command runner failed: {0}")]
    Runner(#[source] CommandRunnerError),
    /// Zot rejected the scoped bearer credential.
    #[error("registry authentication failed")]
    AuthenticationFailed,
    /// Direct bounded registry read-back failed without exposing response data.
    #[error("registry read-back failed")]
    RegistryRead(#[source] RegistryReadError),
    /// The configured registry read origin was not a credential-free HTTP(S) origin.
    #[error("registry read origin is unsafe")]
    UnsafeRegistryOrigin,
    /// A non-authentication OCI client command failed.
    #[error("OCI client command failed")]
    CommandFailed,
    /// Zot did not return a valid top-level descriptor.
    #[error("registry returned a malformed manifest descriptor")]
    MalformedRemoteDescriptor,
    /// Zot returned a descriptor different from the immutable intent.
    #[error("registry returned the wrong manifest descriptor")]
    WrongRemoteDescriptor,
    /// Zot returned a malformed manifest document.
    #[error("registry returned a malformed manifest")]
    MalformedRemoteManifest,
    /// Raw manifest bytes did not match the read-back descriptor.
    #[error("registry manifest bytes do not match their descriptor")]
    WrongRemoteManifestBytes,
    /// Zot returned a malformed or incomplete multi-platform index.
    #[error("registry returned a malformed multi-platform index")]
    MalformedRemoteIndex,
    /// Zot returned malformed referrer discovery data.
    #[error("registry returned malformed referrer discovery data")]
    MalformedReferrers,
    /// Required evidence was absent or duplicated in referrer discovery.
    #[error("registry referrer evidence is missing or duplicated: {0:?}")]
    MissingOrDuplicateReferrer(SupplyChainReferrerKind),
    /// A discovered evidence manifest did not link to the immutable subject.
    #[error("registry referrer points to another subject")]
    WrongReferrerSubject,
    /// A referrer descriptor or manifest differed from the discovered evidence.
    #[error("registry referrer descriptor or manifest is inconsistent")]
    WrongReferrerDescriptor,
    /// A domain invariant rejected local or remote data.
    #[error("registry domain verification failed: {0}")]
    Domain(#[source] RegistryValueError),
}
