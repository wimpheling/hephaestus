use super::prelude::*;
use super::{
    notification::{
        ClaimedRegistryNotification, RegistryNotificationAction, RegistryNotificationReceipt,
        RegistryNotificationTarget, RegistryStoreError,
    },
    rows::NotificationRow,
};
pub(super) fn owner_fields(
    owner: &RegistryOwner,
) -> (&'static str, Option<&str>, Option<Uuid>, Option<Uuid>) {
    match owner {
        RegistryOwner::PlatformImage { image_key } => {
            ("platform_image", Some(image_key.as_str()), None, None)
        }
        RegistryOwner::RepositoryOciImage {
            project_id,
            image_id,
        } => (
            "repository_oci_image",
            None,
            Some(image_id.as_uuid()),
            Some(project_id.as_uuid()),
        ),
        RegistryOwner::ReleaseAgent {
            project_id,
            release_agent_id,
        } => (
            "release_agent",
            None,
            Some(release_agent_id.as_uuid()),
            Some(project_id.as_uuid()),
        ),
    }
}

pub(super) fn same_intent_identity(left: &PublicationIntent, right: &PublicationIntent) -> bool {
    // The caller creates a fresh ID for each submission. When the unique
    // durable identity already exists, `create_intent` must return that row
    // so an interrupted publication can be retried; the generated request ID
    // is deliberately not part of the idempotency comparison.
    left.claim() == right.claim()
        && left.reference() == right.reference()
        && left.expected_manifest() == right.expected_manifest()
        && left.policy_version() == right.policy_version()
        && left.supply_chain_policy() == right.supply_chain_policy()
}

pub(super) fn descriptor(
    digest: Sha256Digest,
    size: i64,
    media_type: String,
) -> Result<OciDescriptor, RegistryStoreError> {
    let size = u64::try_from(size).map_err(|_| RegistryStoreError::InvalidStoredData)?;
    Ok(OciDescriptor::new(
        digest,
        size,
        OciMediaType::parse(media_type)?,
    )?)
}

pub(super) fn parse_referrer_kind(
    value: &str,
) -> Result<SupplyChainReferrerKind, RegistryStoreError> {
    match value {
        "sbom" => Ok(SupplyChainReferrerKind::Sbom),
        "provenance" => Ok(SupplyChainReferrerKind::Provenance),
        "scan" => Ok(SupplyChainReferrerKind::Scan),
        "signature" => Ok(SupplyChainReferrerKind::Signature),
        _ => Err(RegistryStoreError::InvalidStoredData),
    }
}

pub(super) const fn referrer_kind_text(value: SupplyChainReferrerKind) -> &'static str {
    match value {
        SupplyChainReferrerKind::Sbom => "sbom",
        SupplyChainReferrerKind::Provenance => "provenance",
        SupplyChainReferrerKind::Scan => "scan",
        SupplyChainReferrerKind::Signature => "signature",
    }
}

pub(super) fn parse_state(value: &str) -> Result<PublicationState, RegistryStoreError> {
    match value {
        "pending" => Ok(PublicationState::Pending),
        "publishing" => Ok(PublicationState::Publishing),
        "verified" => Ok(PublicationState::Verified),
        "approved" => Ok(PublicationState::Approved),
        "retired" => Ok(PublicationState::Retired),
        "missing" => Ok(PublicationState::Missing),
        _ => Err(RegistryStoreError::InvalidStoredData),
    }
}

pub(super) const fn state_text(value: PublicationState) -> &'static str {
    match value {
        PublicationState::Pending => "pending",
        PublicationState::Publishing => "publishing",
        PublicationState::Verified => "verified",
        PublicationState::Approved => "approved",
        PublicationState::Retired => "retired",
        PublicationState::Missing => "missing",
    }
}

impl NotificationRow {
    pub(super) fn try_into_receipt(
        self,
    ) -> Result<RegistryNotificationReceipt, RegistryStoreError> {
        let _ = self.try_into_claimed_parts()?;
        Ok(RegistryNotificationReceipt {
            id: self.id,
            event_key: self.event_key,
            duplicate: false,
        })
    }

    pub(super) fn try_into_claimed(
        self,
    ) -> Result<ClaimedRegistryNotification, RegistryStoreError> {
        let (repository_path, namespace, action, target) = self.try_into_claimed_parts()?;
        Ok(ClaimedRegistryNotification {
            id: self.id,
            claim_token: self
                .claim_token
                .ok_or(RegistryStoreError::InvalidStoredData)?,
            repository_path,
            namespace,
            action,
            target,
            occurred_at: self.event_occurred_at,
        })
    }

    fn try_into_claimed_parts(
        &self,
    ) -> Result<
        (
            String,
            Option<RegistryNamespace>,
            RegistryNotificationAction,
            Option<RegistryNotificationTarget>,
        ),
        RegistryStoreError,
    > {
        if self.payload_sha256.len() != 32
            || (self.state == "claimed" && self.lease_expires_at.is_none())
            || ((self.state == "processed" || self.state == "rejected")
                && self.processed_at.is_none())
            || (self.state == "rejected" && self.failure_code.is_none())
        {
            return Err(RegistryStoreError::InvalidStoredData);
        }
        let target = match (
            &self.target_digest,
            &self.target_media_type,
            self.target_size,
        ) {
            (None, None, None) => None,
            (Some(digest), Some(media_type), None) => Some(RegistryNotificationTarget {
                digest: Sha256Digest::parse(digest.clone())?,
                media_type: OciMediaType::parse(media_type.clone())?,
            }),
            (Some(digest), Some(media_type), Some(size)) => {
                let descriptor = descriptor(
                    Sha256Digest::parse(digest.clone())?,
                    size,
                    media_type.clone(),
                )?;
                Some(RegistryNotificationTarget {
                    digest: descriptor.digest().clone(),
                    media_type: descriptor.media_type().clone(),
                })
            }
            _ => return Err(RegistryStoreError::InvalidStoredData),
        };
        if !valid_observed_repository_path(&self.repository_path) {
            return Err(RegistryStoreError::InvalidStoredData);
        }
        Ok((
            self.repository_path.clone(),
            RegistryNamespace::parse(self.repository_path.clone()).ok(),
            RegistryNotificationAction::parse(&self.action)?,
            target,
        ))
    }
}

pub(super) fn valid_observed_repository_path(value: &str) -> bool {
    (1..=255).contains(&value.len())
        && value.split('/').all(|component| {
            !component.is_empty()
                && component.len() <= 128
                && component.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
                })
                && !component.ends_with(['.', '_', '-'])
        })
}

pub(super) fn validate_failure_code(value: &str) -> Result<(), RegistryStoreError> {
    (!value.is_empty()
        && value.len() <= 64
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (byte == b'_' && index > 0)
        }))
    .then_some(())
    .ok_or(RegistryStoreError::InvalidNotification)
}

pub(super) fn storage(error: impl std::error::Error + Send + Sync + 'static) -> RegistryStoreError {
    RegistryStoreError::Storage(Box::new(error))
}

pub(super) fn count_to_u64(value: i64) -> Result<u64, RegistryStoreError> {
    u64::try_from(value).map_err(|_| RegistryStoreError::InvalidStoredData)
}
