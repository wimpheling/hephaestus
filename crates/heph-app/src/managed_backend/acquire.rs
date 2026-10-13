//! Pin both independent physical roots before either prospective publication.

use super::{AppError, ManagedBackendConfig, ManagedRootIntent, custody};
use std::{error::Error, fmt, sync::Arc};
use vm_libkrun::{LibkrunProvider, ProspectiveOwnedVmBootstrap};
use volume_local::{ProspectiveVolumeRootBootstrap, VolumeRootOwner};

pub(super) enum PreparedVm {
    Existing(LibkrunProvider),
    Bootstrap(ProspectiveOwnedVmBootstrap),
}

enum PreparedVolume {
    Existing(VolumeRootOwner),
    Bootstrap(ProspectiveVolumeRootBootstrap),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum AcquisitionPhase {
    PrepareVm,
    PrepareVolume,
    PublishVolume,
    PublishVm,
}

#[derive(Debug, thiserror::Error)]
#[error("managed acquisition remains held during {phase:?}: {source}")]
pub(super) struct AcquisitionFailure {
    phase: AcquisitionPhase,
    #[source]
    source: AppError,
}

const fn failed(phase: AcquisitionPhase, source: AppError) -> AcquisitionFailure {
    AcquisitionFailure { phase, source }
}

pub(super) fn owners(
    config: &ManagedBackendConfig,
) -> Result<(Arc<LibkrunProvider>, Arc<VolumeRootOwner>), AcquisitionFailure> {
    let scope = config.configuration.provider_scope();
    let vm = match config.vm_intent {
        ManagedRootIntent::BootstrapEmpty => PreparedVm::Bootstrap(
            LibkrunProvider::prepare_owned_bootstrap(config.provider.clone(), scope)
                .map_err(|cause| failed(AcquisitionPhase::PrepareVm, custody::failure(cause)))?,
        ),
        ManagedRootIntent::OpenExisting => PreparedVm::Existing(
            LibkrunProvider::open_owned(config.provider.clone(), scope)
                .map_err(|cause| failed(AcquisitionPhase::PrepareVm, custody::failure(cause)))?,
        ),
    };
    let volume = match config.volume_intent {
        ManagedRootIntent::BootstrapEmpty => VolumeRootOwner::prepare_empty_with_namespace(
            &config.volumes.volume_root,
            &config.volumes.host_id,
            config.configuration.volume_namespace(),
        )
        .map(PreparedVolume::Bootstrap),
        ManagedRootIntent::OpenExisting => VolumeRootOwner::open_existing(
            &config.volumes.volume_root,
            &config.volumes.host_id,
            config.configuration.volume_namespace(),
        )
        .map(PreparedVolume::Existing),
    };
    let volume = match volume {
        Ok(volume) => volume,
        Err(cause) => {
            return Err(failed(
                AcquisitionPhase::PrepareVolume,
                held(cause, Some(vm), None),
            ));
        }
    };
    // Both original physical acquisitions completed without owner publication.
    // Each consuming publish rechecks its pinned roots; this is not a dual-FS
    // commit or a fence against filesystem races or subsequent database births.
    let volume = match volume {
        PreparedVolume::Existing(owner) => owner,
        PreparedVolume::Bootstrap(prepared) => match prepared.publish() {
            Ok(owner) => owner,
            Err(cause) => {
                return Err(failed(
                    AcquisitionPhase::PublishVolume,
                    held(cause, Some(vm), None),
                ));
            }
        },
    };
    let volume = Arc::new(volume);
    match vm {
        PreparedVm::Existing(provider) => Ok((Arc::new(provider), volume)),
        PreparedVm::Bootstrap(prepared) => match prepared.publish() {
            Ok(provider) => Ok((Arc::new(provider), volume)),
            // Preserve the actual late VmError with its private original FD/lock
            // custody, and retain the first published/opened volume owner too.
            Err(cause) => Err(failed(
                AcquisitionPhase::PublishVm,
                held(cause, None, Some(volume)),
            )),
        },
    }
}

fn held(
    cause: impl Error + Send + Sync + 'static,
    vm: Option<PreparedVm>,
    volume: Option<Arc<VolumeRootOwner>>,
) -> AppError {
    custody::failure(HeldAcquisition {
        cause: Box::new(cause),
        _vm: vm,
        _volume: volume,
    })
}

struct HeldAcquisition {
    cause: Box<dyn Error + Send + Sync>,
    _vm: Option<PreparedVm>,
    _volume: Option<Arc<VolumeRootOwner>>,
}

impl fmt::Debug for HeldAcquisition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HeldAcquisition")
            .field("cause", &self.cause)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for HeldAcquisition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "managed root acquisition remains held: {}",
            self.cause
        )
    }
}

impl Error for HeldAcquisition {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
