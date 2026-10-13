//! Complete immutable comparison observations; no bootstrap authority or fence.

use super::{AppError, ManagedBackendConfig, ManagedRootIntent, custody, invalid};
use control_plane_postgres::ControlPlanePool;
use run_domain::VmOwnerHistory;
use run_postgres::PgVmOwnerHistoryReader;
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{OwnedVolumeRootHistory, VolumeRootHistoryRepository};

pub(super) async fn observe(
    config: &ManagedBackendConfig,
    worker: &ControlPlanePool,
) -> Result<(), AppError> {
    let vm = PgVmOwnerHistoryReader::new(worker.clone())
        .observe(config.configuration.provider_scope())
        .await
        .map_err(custody::failure)?;
    check_vm(config.vm_intent, vm)?;
    let volume = PostgresVolumeMetadataRepository::new(worker.clone())
        .owned_root_history(&config.volumes.host_id, &config.volumes.volume_root)
        .await
        .map_err(custody::failure)?;
    check_volume(
        config.volume_intent,
        config.configuration.volume_namespace(),
        volume,
    )
}

pub(super) fn check_vm(intent: ManagedRootIntent, history: VmOwnerHistory) -> Result<(), AppError> {
    match (intent, history) {
        (_, VmOwnerHistory::NoHistoryForConfiguredHost)
        | (ManagedRootIntent::OpenExisting, VmOwnerHistory::Exact) => Ok(()),
        // Unknown global originals hold even when claimed unrelated to this host.
        // No filtered recovery/active-state absence or adoption fallback exists.
        _ => Err(invalid(
            "VM original history conflicts, is unknown, or forbids bootstrap",
        )),
    }
}

pub(super) fn check_volume(
    intent: ManagedRootIntent,
    expected: volume_trait::VolumeRootNamespaceId,
    history: OwnedVolumeRootHistory,
) -> Result<(), AppError> {
    match history {
        OwnedVolumeRootHistory::NoOwnedHistory => Ok(()),
        OwnedVolumeRootHistory::Existing { namespace }
            if intent == ManagedRootIntent::OpenExisting && namespace == expected =>
        {
            Ok(())
        }
        _ => Err(invalid(
            "volume original history conflicts or forbids bootstrap",
        )),
    }
}
