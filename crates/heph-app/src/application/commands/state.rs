//! Shared state for internal application commands.

use release_domain::RuntimePolicy;
use release_postgres::ReleaseService;
use secret_postgres::SecretService;
use secret_store::LocalKeyProvider;
use std::sync::Arc;

#[derive(Clone)]
pub struct InternalCommandState {
    pub(super) releases: Arc<ReleaseService>,
    pub(super) secrets: Arc<SecretService<LocalKeyProvider>>,
    pub(super) platform_policy: RuntimePolicy,
    pub(super) platform_policy_version: String,
}

impl InternalCommandState {
    pub(crate) const fn new(
        releases: Arc<ReleaseService>,
        secrets: Arc<SecretService<LocalKeyProvider>>,
        platform_policy: RuntimePolicy,
        platform_policy_version: String,
    ) -> Self {
        Self {
            releases,
            secrets,
            platform_policy,
            platform_policy_version,
        }
    }
}
