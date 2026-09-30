use super::notification::RegistryStoreError;
use super::prelude::*;
use super::{
    parsing::{descriptor, parse_referrer_kind, parse_state, referrer_kind_text, storage},
    rows::{EvidenceRow, PlatformRow, PublicationRow},
};
pub(super) fn publication_from_rows(
    row: PublicationRow,
    platforms: Vec<PlatformRow>,
    evidence: Vec<EvidenceRow>,
) -> Result<PublicationIntent, RegistryStoreError> {
    let owner = match row.owner_kind.as_str() {
        "platform_image" => RegistryOwner::PlatformImage {
            image_key: PlatformImageKey::parse(
                row.platform_image_key
                    .ok_or(RegistryStoreError::InvalidStoredData)?,
            )?,
        },
        "repository_oci_image" => RegistryOwner::RepositoryOciImage {
            project_id: forge_domain::ProjectId::from_uuid(
                row.project_id
                    .ok_or(RegistryStoreError::InvalidStoredData)?,
            ),
            image_id: builder_catalog_domain::OciImageId::from_uuid(
                row.owner_id.ok_or(RegistryStoreError::InvalidStoredData)?,
            ),
        },
        "release_agent" => RegistryOwner::ReleaseAgent {
            project_id: forge_domain::ProjectId::from_uuid(
                row.project_id
                    .ok_or(RegistryStoreError::InvalidStoredData)?,
            ),
            release_agent_id: runtime_types::ReleaseAgentId::from_uuid(
                row.owner_id.ok_or(RegistryStoreError::InvalidStoredData)?,
            ),
        },
        _ => return Err(RegistryStoreError::InvalidStoredData),
    };
    let claim = NamespaceClaim::new(owner);
    if claim.namespace().as_str() != row.repository_path {
        return Err(RegistryStoreError::InvalidStoredData);
    }
    let digest = Sha256Digest::parse(row.expected_digest)?;
    let reference = ImmutableManifestReference::new(
        RegistryAuthority::parse(row.registry_authority)?,
        RegistryNamespace::parse(row.repository_path)?,
        digest.clone(),
    );
    let expected = descriptor(digest, row.expected_size, row.expected_media_type)?;
    let mut intent = PublicationIntent::new(
        PublicationIntentId::from_uuid(row.id),
        claim,
        reference,
        expected.clone(),
        PolicyVersion::parse(row.policy_version)?,
        if row.signature_required {
            SupplyChainPolicy::with_signature()
        } else {
            SupplyChainPolicy::without_signature()
        },
    )?;
    let state = parse_state(&row.state)?;
    if matches!(state, PublicationState::Publishing) {
        intent = intent
            .begin_publishing()
            .map_err(RegistryStoreError::Lifecycle)?;
    }
    if matches!(
        state,
        PublicationState::Verified | PublicationState::Approved | PublicationState::Missing
    ) || (state == PublicationState::Retired && row.verified_at.is_some())
    {
        let verification = verification_from_rows(
            &reference_from_intent(&intent),
            expected,
            platforms,
            evidence,
        )?;
        intent = intent
            .record_verified(verification)
            .map_err(RegistryStoreError::Lifecycle)?;
        if matches!(
            state,
            PublicationState::Approved | PublicationState::Missing
        ) || (state == PublicationState::Retired && row.approved_at.is_some())
        {
            intent = intent.approve().map_err(RegistryStoreError::Lifecycle)?;
        }
        if state == PublicationState::Missing {
            intent = intent
                .mark_missing()
                .map_err(RegistryStoreError::Lifecycle)?;
        }
    }
    if state == PublicationState::Retired {
        intent = intent.retire();
    }
    Ok(intent)
}

fn reference_from_intent(intent: &PublicationIntent) -> ImmutableManifestReference {
    intent.reference().clone()
}

pub(super) fn verification_from_rows(
    reference: &ImmutableManifestReference,
    manifest: OciDescriptor,
    platforms: Vec<PlatformRow>,
    evidence: Vec<EvidenceRow>,
) -> Result<VerifiedPublication, RegistryStoreError> {
    let platforms = platforms
        .into_iter()
        .map(|row| {
            let descriptor =
                descriptor(Sha256Digest::parse(row.digest)?, row.size, row.media_type)?;
            PlatformDescriptor::new(
                descriptor,
                row.operating_system,
                row.architecture,
                row.variant,
            )
            .map_err(RegistryStoreError::InvalidValue)
        })
        .collect::<Result<Vec<_>, RegistryStoreError>>()?;
    let referrers = evidence
        .into_iter()
        .map(|row| {
            Ok(SupplyChainReferrer::new(
                parse_referrer_kind(&row.kind)?,
                Sha256Digest::parse(row.subject_digest)?,
                descriptor(Sha256Digest::parse(row.digest)?, row.size, row.media_type)?,
                OciMediaType::parse(row.artifact_type)?,
            ))
        })
        .collect::<Result<Vec<_>, RegistryStoreError>>()?;
    let evidence = SupplyChainEvidence::new(reference.digest().clone(), referrers)?;
    Ok(VerifiedPublication::new(
        reference, manifest, platforms, evidence,
    )?)
}

pub(super) async fn insert_verification(
    transaction: &mut Transaction<'_, Postgres>,
    publication_id: Uuid,
    verification: &VerifiedPublication,
) -> Result<(), RegistryStoreError> {
    for platform in verification.platforms() {
        sqlx::query(
            "INSERT INTO registry_publication_platforms (
                publication_id, digest, size, media_type, operating_system, architecture, variant
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(publication_id)
        .bind(platform.descriptor().digest().as_str())
        .bind(
            i64::try_from(platform.descriptor().size())
                .map_err(|_| RegistryStoreError::Conflict)?,
        )
        .bind(platform.descriptor().media_type().as_str())
        .bind(platform.operating_system())
        .bind(platform.architecture())
        .bind(platform.variant())
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
    }
    for referrer in verification.evidence().referrers() {
        sqlx::query(
            "INSERT INTO registry_publication_evidence (
                publication_id, kind, subject_digest, digest, size, media_type, artifact_type
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(publication_id)
        .bind(referrer_kind_text(referrer.kind()))
        .bind(referrer.subject().as_str())
        .bind(referrer.descriptor().digest().as_str())
        .bind(
            i64::try_from(referrer.descriptor().size())
                .map_err(|_| RegistryStoreError::Conflict)?,
        )
        .bind(referrer.descriptor().media_type().as_str())
        .bind(referrer.artifact_type().as_str())
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
    }
    Ok(())
}
