use super::{OwnedVolumeMetadata, filesystem};
use crate::{
    LocalVolumeStore, backing,
    owned_journal::{FormatState, JournalPurpose, OwnedJournal},
};
use identity_domain::AuthenticatedIdentity;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingPhase, OwnedProvisioningContext, VolumeError,
};

pub async fn provision(
    store: &LocalVolumeStore,
    metadata: &OwnedVolumeMetadata,
    identity: &AuthenticatedIdentity,
    request: &BeginOwnedProvisioning,
    current: OwnedProvisioningContext,
) -> Result<OwnedProvisioningContext, VolumeError> {
    // Ready is immutable historical birth evidence, not a new filesystem probe or
    // permission to run fsck against an attached runtime volume.
    if ready(&current) {
        return Ok(current);
    }
    let mut lock = metadata
        .owner
        .journal_lock(journal_purpose(&current)?, false)?;
    metadata.owner.validate()?;
    // Cached pre-lock admission cannot authorize namespace/allocation writes.
    // Recheck live original actor plus permanent birth fence under the actual
    // same-volume flock, before even open() may prepare journal metadata.
    fresh_actor(metadata, identity, request, &current).await?;
    metadata.owner.validate()?;
    let mut journal = if current.observation.is_some() {
        OwnedJournal::open_existing(&mut lock).map_err(backing)?
    } else {
        OwnedJournal::open(&mut lock).map_err(backing)?
    };
    if let Some(previous) = &current.observation {
        journal.validate_history(previous).map_err(backing)?;
    }
    if journal.format_state().map_err(backing)? == FormatState::MayHaveStarted {
        return finish_readonly(store, metadata, &journal, current).await;
    }
    fresh_actor(metadata, identity, request, &current).await?;
    metadata.owner.validate()?;
    let backing = if journal.has_recorded_inode() {
        journal.recover_recorded().map_err(backing)?
    } else {
        journal.create_staging().map_err(backing)?
    };
    allocate(&journal, &backing, &current)?;
    journal.publish(&backing).map_err(crate::backing)?;
    let observed = journal
        .observe(&backing, &current.claim, None)
        .map_err(crate::backing)?;
    metadata
        .worker
        .record_owned_observation(&current.claim, &observed)
        .await?;
    format(
        store, metadata, identity, request, &current, &journal, &backing,
    )
    .await?;
    finish_readonly(store, metadata, &journal, current).await
}
fn allocate(
    journal: &OwnedJournal<'_>,
    backing: &crate::owned_journal::RecordedBacking,
    current: &OwnedProvisioningContext,
) -> Result<(), VolumeError> {
    filesystem::no_unjournaled_format(backing.file())?;
    if !journal.allocation_complete().map_err(crate::backing)? {
        journal.begin_allocation(backing).map_err(crate::backing)?;
        let capacity = current
            .claim
            .purpose
            .receipt()
            .intent
            .registration()
            .capacity_bytes();
        match backing.file().metadata().map_err(crate::backing)?.len() {
            0 => backing.file().set_len(capacity).map_err(crate::backing)?,
            size if size == capacity => {} // Exact claimed set_len→record gap: sync, never resize.
            _ => {
                return Err(VolumeError::ProvisioningUncertain(
                    "claimed allocation has a contradictory length",
                ));
            }
        }
        journal.mark_allocated(backing).map_err(crate::backing)?;
    }
    Ok(())
}
async fn format(
    store: &LocalVolumeStore,
    metadata: &OwnedVolumeMetadata,
    identity: &AuthenticatedIdentity,
    request: &BeginOwnedProvisioning,
    current: &OwnedProvisioningContext,
    journal: &OwnedJournal<'_>,
    backing: &crate::owned_journal::RecordedBacking,
) -> Result<(), VolumeError> {
    // Historical admission is insufficient: replay checks fresh middleware identity
    // and original actor's current ProjectManage immediately before format intent.
    fresh_actor(metadata, identity, request, current).await?;
    metadata.owner.validate()?;
    journal
        .begin_first_format(backing)
        .map_err(crate::backing)?;
    let format_intent = journal
        .observe(backing, &current.claim, None)
        .map_err(crate::backing)?;
    metadata
        .worker
        .record_owned_observation(&current.claim, &format_intent)
        .await?;
    // Recording can await across revocation. Nothing asynchronous intervenes
    // between this fresh admission and actual formatter spawning.
    fresh_actor(metadata, identity, request, current).await?;
    metadata.owner.validate()?;
    let status = filesystem::locked_command(
        &store.config.mkfs_ext4,
        journal.child_lock().map_err(crate::backing)?,
    )
    .args(["-q", "-F", "-b", "4096", "-U"])
    .arg(
        current
            .claim
            .purpose
            .receipt()
            .intent
            .registration()
            .filesystem_uuid()
            .to_string(),
    )
    .arg(filesystem::descriptor(backing.file()))
    .status()
    .await
    .map_err(crate::backing)?;
    if !status.success() {
        return Err(VolumeError::ProvisioningUncertain(
            "owned first format did not complete; never repeat format",
        ));
    }
    backing.file().sync_all().map_err(crate::backing)?;
    Ok(())
}
pub async fn reconcile(
    store: &LocalVolumeStore,
    metadata: &OwnedVolumeMetadata,
    current: OwnedProvisioningContext,
) -> Result<OwnedProvisioningContext, VolumeError> {
    if ready(&current) {
        return Ok(current);
    }
    let mut lock = metadata
        .owner
        .journal_lock(journal_purpose(&current)?, true)?;
    metadata.owner.validate()?;
    let journal = OwnedJournal::open_existing(&mut lock).map_err(backing)?;
    if let Some(previous) = &current.observation {
        journal.validate_history(previous).map_err(backing)?;
    }
    finish_readonly(store, metadata, &journal, current).await
}
async fn finish_readonly(
    store: &LocalVolumeStore,
    metadata: &OwnedVolumeMetadata,
    journal: &OwnedJournal<'_>,
    current: OwnedProvisioningContext,
) -> Result<OwnedProvisioningContext, VolumeError> {
    metadata.owner.validate()?;
    if !journal.allocation_complete().map_err(backing)? {
        return Err(VolumeError::ProvisioningUncertain(
            "owned birth lacks completed allocation; readonly recovery cannot allocate",
        ));
    }
    if !journal.publication_complete().map_err(backing)? {
        return Err(VolumeError::ProvisioningUncertain(
            "owned birth lacks completed publication; readonly recovery cannot publish",
        ));
    }
    if !journal.format_intent_present().map_err(backing)? {
        return Err(VolumeError::ProvisioningUncertain(
            "allocated owned birth never started format; readonly recovery cannot first-format",
        ));
    }
    let backing = journal.recover_readonly().map_err(crate::backing)?;
    let registration = current.claim.purpose.receipt().intent.registration();
    let birth = filesystem::prove(
        backing.file(),
        journal.child_lock().map_err(crate::backing)?,
        registration.capacity_bytes(),
        registration.filesystem_uuid(),
        &store.config.mkfs_ext4,
    )
    .await?;
    metadata.owner.validate()?;
    let observed = journal
        .observe(&backing, &current.claim, Some(birth))
        .map_err(crate::backing)?;
    metadata
        .worker
        .record_owned_observation(&current.claim, &observed)
        .await
}
async fn fresh_actor(
    metadata: &OwnedVolumeMetadata,
    identity: &AuthenticatedIdentity,
    request: &BeginOwnedProvisioning,
    current: &OwnedProvisioningContext,
) -> Result<(), VolumeError> {
    let replay = metadata
        .actor
        .begin_owned_provisioning(identity, request)
        .await?;
    if replay.claim != current.claim || ready(&replay) {
        return Err(VolumeError::IntentConflict);
    }
    Ok(())
}
fn journal_purpose(current: &OwnedProvisioningContext) -> Result<JournalPurpose, VolumeError> {
    let purpose = &current.claim.purpose;
    JournalPurpose::new(
        purpose.receipt(),
        purpose.host(),
        purpose.root(),
        purpose.owner_namespace(),
        purpose.birth_generation(),
    )
    .map_err(backing)
}
fn ready(current: &OwnedProvisioningContext) -> bool {
    current
        .observation
        .as_ref()
        .is_some_and(|observed| observed.fields().phase == OwnedBackingPhase::Ready)
}
