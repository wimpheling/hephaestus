//! Ordinary secret services with actual distinct application and worker pools.

use super::{AppError, ManagedLegacyInputs, component};
use secret_postgres::{SecretRuntimeService, SecretService, initialize_manager};
use secret_runtime::FilesystemSecretMountProvider;
use secret_store::EncryptedStore;
use std::sync::Arc;

pub(super) fn build(
    inputs: &ManagedLegacyInputs,
) -> Result<Arc<dyn heph_run::RunSecretManager>, AppError> {
    let authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
    let dispatch = SecretService::new(
        inputs.pools.application.clone(),
        EncryptedStore::new(inputs.secret_keys.clone()),
        authorizer.clone(),
    );
    let runtime = SecretRuntimeService::new(
        inputs.pools.application.clone(),
        inputs.pools.worker.clone(),
        EncryptedStore::new(inputs.secret_keys.clone()),
        authorizer,
    )
    .with_mount_provider(Arc::new(FilesystemSecretMountProvider::new()));
    let manager = initialize_manager(
        inputs.pools.worker.clone(),
        dispatch,
        runtime,
        inputs.secret_mounts.clone(),
    )
    .map_err(component("managed Legacy ordinary secret manager"))?;
    Ok(Arc::new(manager))
}
