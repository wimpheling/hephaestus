//! Transport-independent gateway installation use cases.

use forge_service::GitStorage;
use gateway_postgres::{GatewayInstallError, InstallGatewayManifest, PostgresGatewayInstaller};
use identity_domain::AuthenticatedIdentity;
use release_domain::ReleaseId;
use std::sync::Arc;

const MANIFEST_PATH: &str = "heph.gateways.toml";
const MAX_MANIFEST_BYTES: usize = 1_048_576;

/// Application use case for installing declarations from one immutable
/// published gateway release.
#[derive(Clone)]
pub struct GatewayInstallApplication {
    installer: PostgresGatewayInstaller,
    storage: Arc<GitStorage>,
}

impl GatewayInstallApplication {
    /// Creates the use case over the gateway adapter and canonical Git source.
    #[must_use]
    pub const fn new(installer: PostgresGatewayInstaller, storage: Arc<GitStorage>) -> Self {
        Self { installer, storage }
    }

    /// Retrieves the exact release-root manifest and installs it atomically.
    ///
    /// # Errors
    ///
    /// Returns a safe release, source, authorization, manifest, or persistence
    /// failure.
    pub async fn install_release(
        &self,
        identity: &AuthenticatedIdentity,
        release_id: ReleaseId,
    ) -> Result<(), GatewayInstallError> {
        let source = self
            .installer
            .published_release(identity, release_id)
            .await?;
        let manifest = self
            .storage
            .read_file_at_commit(
                source.repository_id,
                &source.source_commit,
                MANIFEST_PATH,
                MAX_MANIFEST_BYTES,
            )
            .await
            .map_err(|_| GatewayInstallError::Unavailable)?;
        self.installer
            .install(
                identity,
                InstallGatewayManifest {
                    project_id: source.project_id,
                    repository_id: source.repository_id,
                    release_id: Some(source.release_id),
                    manifest,
                },
            )
            .await?;
        Ok(())
    }
}
