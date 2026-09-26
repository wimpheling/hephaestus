//! Compile-time contract coverage for the provider-neutral secret facade.

use std::{marker::PhantomData, path::PathBuf};

use heph_secret::{
    EphemeralSecretConfig, MaterializedSecretMount, RawSecretFile, SecretDispatchInput,
    SecretMountManager, SecretMountMetadata, SecretMountProvider, SecretRuntimeError,
};

fn accepts_provider_contract(
    provider: &dyn SecretMountProvider,
    config: &EphemeralSecretConfig,
) -> Result<(), SecretRuntimeError> {
    provider.validate_config(config)
}

const fn manager_type_is_public<M, D, R>() -> PhantomData<SecretMountManager<M, D, R>> {
    PhantomData
}

#[test]
fn facade_exports_only_provider_neutral_contract_types() {
    let _config = EphemeralSecretConfig {
        root: PathBuf::from("/unused"),
        require_memory_filesystem: false,
    };
    let _ = accepts_provider_contract;

    let _: Option<MaterializedSecretMount> = None;
    let _: Option<RawSecretFile> = None;
    let _: Option<SecretDispatchInput> = None;
    let _: Option<&dyn SecretMountMetadata> = None;
    let _: PhantomData<SecretMountManager<(), (), ()>> = manager_type_is_public();
}
