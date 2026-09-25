use registry_domain::RegistryAuthority;
use reqwest::Url;
use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
};

use super::{
    errors::{CommandRunnerError, PublisherError},
    paths::{canonical_directory, canonical_executable, parse_registry_origin, registry_origin},
};

/// Immutable, administrator-owned locations and binaries for OCI publication.
#[derive(Debug, Clone)]
pub struct PublisherConfiguration {
    pub(super) authority: RegistryAuthority,
    pub(super) layout_root: PathBuf,
    pub(super) evidence_root: PathBuf,
    pub(super) credential_root: PathBuf,
    pub(super) skopeo_binary: PathBuf,
    pub(super) registry_origin: Url,
}

impl PublisherConfiguration {
    /// Validates the administrator-controlled publisher boundary.
    ///
    /// # Errors
    ///
    /// Returns an error when a root or executable is relative, missing, or a
    /// symbolic link. The publication inputs must be descendants of
    /// `layout_root`; verifier evidence must be below `evidence_root`;
    /// temporary credential files are created below
    /// `credential_root` and removed before this adapter returns.
    pub fn new(
        authority: RegistryAuthority,
        layout_root: &Path,
        evidence_root: &Path,
        credential_root: &Path,
        skopeo_binary: &Path,
        _oras_binary: &Path,
    ) -> Result<Self, PublisherError> {
        let registry_origin = registry_origin(&authority)?;
        Ok(Self {
            authority,
            layout_root: canonical_directory(layout_root)?,
            evidence_root: canonical_directory(evidence_root)?,
            credential_root: canonical_directory(credential_root)?,
            skopeo_binary: canonical_executable(skopeo_binary)?,
            registry_origin,
        })
    }

    /// Returns the configured, fixed registry authority.
    #[must_use]
    pub const fn authority(&self) -> &RegistryAuthority {
        &self.authority
    }

    /// Replaces the registry read origin for an explicitly configured private
    /// endpoint. This supports local loopback Zot without weakening the
    /// default HTTPS origin derived from the public authority.
    ///
    /// # Errors
    ///
    /// Returns an error unless `origin` is a credential-free HTTP(S) origin.
    pub fn with_registry_origin(mut self, origin: &str) -> Result<Self, PublisherError> {
        self.registry_origin = parse_registry_origin(origin)?;
        Ok(self)
    }
}

/// Evidence files produced by the trusted build and scanning stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationEvidenceFiles {
    /// SPDX JSON software bill of materials.
    pub sbom: PathBuf,
    /// In-toto build provenance statement.
    pub provenance: PathBuf,
    /// Trusted vulnerability scan result.
    pub scan: PathBuf,
    /// Optional signing or approval artifact.
    pub signature: Option<PathBuf>,
}

/// The local, administrator-owned publication material for one durable intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationMaterial {
    /// OCI image layout directory, below the configured layout root.
    pub layout: PathBuf,
    /// Required and optional supply-chain evidence files.
    pub evidence: PublicationEvidenceFiles,
}

/// A command to execute without a shell.
#[derive(Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub(super) program: PathBuf,
    pub(super) arguments: Vec<OsString>,
    pub(super) environment: Vec<(OsString, OsString)>,
    pub(super) sensitive_argument_positions: Vec<usize>,
}

impl CommandSpec {
    pub(super) const fn new(program: PathBuf, arguments: Vec<OsString>) -> Self {
        Self {
            program,
            arguments,
            environment: Vec::new(),
            sensitive_argument_positions: Vec::new(),
        }
    }

    pub(super) fn with_sensitive_argument(mut self, position: usize) -> Self {
        self.sensitive_argument_positions.push(position);
        self
    }

    /// Returns the executable path.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Returns literal command arguments.
    #[must_use]
    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// Returns the narrowly scoped subprocess environment.
    #[must_use]
    pub fn environment_entries(&self) -> &[(OsString, OsString)] {
        &self.environment
    }
}

impl fmt::Debug for CommandSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arguments = self
            .arguments
            .iter()
            .enumerate()
            .map(|(position, argument)| {
                if self.sensitive_argument_positions.contains(&position) {
                    String::from("REDACTED")
                } else {
                    argument.to_string_lossy().into_owned()
                }
            })
            .collect::<Vec<_>>();
        formatter
            .debug_struct("CommandSpec")
            .field("program", &self.program)
            .field("arguments", &arguments)
            .field("environment", &"REDACTED")
            .finish()
    }
}

/// Captured result from an injectable command runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// Process exit status as supplied by the runner.
    pub success: bool,
    /// Standard output bytes.
    pub stdout: Vec<u8>,
    /// Standard error bytes, retained only until classified as a failure.
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    /// Creates a successful captured command result.
    #[must_use]
    pub fn success(stdout: impl Into<Vec<u8>>) -> Self {
        Self {
            success: true,
            stdout: stdout.into(),
            stderr: Vec::new(),
        }
    }

    /// Creates an unsuccessful captured command result.
    #[must_use]
    pub fn failure(stderr: impl Into<Vec<u8>>) -> Self {
        Self {
            success: false,
            stdout: Vec::new(),
            stderr: stderr.into(),
        }
    }
}

/// Executes one literal publisher command.
pub trait CommandRunner: Send + Sync {
    /// Executes `command` without inheriting an ambient shell.
    ///
    /// # Errors
    ///
    /// Returns only a non-sensitive process-launch failure.
    fn run(&self, command: &CommandSpec) -> Result<CommandOutput, CommandRunnerError>;
}
