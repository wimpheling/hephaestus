use super::support::*;

pub(super) fn successful_outputs(
    intent: &PublicationIntent,
    material: &PublicationMaterial,
) -> Vec<CommandOutput> {
    successful_outputs_for_subject(
        intent,
        material,
        intent.reference().digest(),
        material.evidence.signature.is_some(),
    )
}

pub(super) fn successful_outputs_for_subject(
    intent: &PublicationIntent,
    material: &PublicationMaterial,
    referrer_subject: &Sha256Digest,
    include_signature: bool,
) -> Vec<CommandOutput> {
    let subject = intent.reference().digest();
    let descriptor = format!(
        r#"{{"mediaType":"{OCI_INDEX_MEDIA_TYPE}","digest":"{subject}","size":{}}}"#,
        intent.expected_manifest().size()
    );
    let index = fs::read(
        material
            .layout
            .join("blobs/sha256")
            .join(subject.as_str().trim_start_matches("sha256:")),
    )
    .expect("subject manifest");
    let mut artifact_types = vec![
        SBOM_ARTIFACT_TYPE,
        PROVENANCE_ARTIFACT_TYPE,
        SCAN_ARTIFACT_TYPE,
    ];
    if include_signature {
        artifact_types.push(SIGNATURE_ARTIFACT_TYPE);
    }
    let referrer_manifests = artifact_types.into_iter()
        .map(|artifact_type| {
            let bytes = format!(
                r#"{{"mediaType":"{OCI_MANIFEST_MEDIA_TYPE}","artifactType":"{artifact_type}","subject":{{"digest":"{referrer_subject}"}}}}"#
            )
            .into_bytes();
            let descriptor = descriptor_for_bytes(&bytes, OCI_MANIFEST_MEDIA_TYPE);
            (artifact_type, descriptor, bytes)
        })
        .collect::<Vec<_>>();
    let referrers = referrer_manifests
        .iter()
        .map(|(artifact_type, descriptor, _)| {
            format!(
                r#"{{"mediaType":"{}","digest":"{}","size":{},"artifactType":"{artifact_type}"}}"#,
                descriptor.media_type(),
                descriptor.digest(),
                descriptor.size()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut outputs = vec![CommandOutput::success(Vec::new())];
    outputs.extend(
        std::iter::repeat_with(|| CommandOutput::success(Vec::new()))
            .take(referrer_manifests.len()),
    );
    outputs.push(CommandOutput::success(descriptor));
    outputs.push(CommandOutput::success(index));
    outputs.push(CommandOutput::success(format!(
        r#"{{"manifests":[{referrers}]}}"#
    )));
    for (_, descriptor, bytes) in referrer_manifests {
        outputs.push(CommandOutput::success(format!(
            r#"{{"mediaType":"{}","digest":"{}","size":{}}}"#,
            descriptor.media_type(),
            descriptor.digest(),
            descriptor.size()
        )));
        outputs.push(CommandOutput::success(bytes));
    }
    outputs
}

pub(super) fn descriptor_for_bytes(bytes: &[u8], media_type: &str) -> OciDescriptor {
    OciDescriptor::new(
        Sha256Digest::parse(format!("sha256:{:x}", Sha256::digest(bytes))).expect("digest"),
        u64::try_from(bytes.len()).expect("size"),
        OciMediaType::parse(media_type).expect("media type"),
    )
    .expect("descriptor")
}
