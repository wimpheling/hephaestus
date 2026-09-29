use super::{outputs::*, support::*};

#[test]
fn publishes_by_intent_digest_and_returns_verified_evidence() {
    let (_root, config, material, intent) = setup();
    let runner = ScriptedRunner::with(successful_outputs(&intent, &material));
    let publisher = scripted_publisher(config, runner);
    let token = token();
    let verified = publisher
        .publish(&intent, &material, token.token())
        .expect("verified");
    assert_eq!(verified.manifest(), intent.expected_manifest());
    assert_eq!(verified.platforms().len(), 1);
    assert_eq!(verified.evidence().referrers().len(), 3);
}

#[test]
fn verifies_an_optional_signature_referrer_when_it_is_published() {
    let (_root, config, mut material, intent) = setup();
    material.evidence.signature = Some(write_file(
        material.evidence.sbom.parent().expect("evidence parent"),
        "signature.json",
        b"signature",
    ));
    let runner = ScriptedRunner::with(successful_outputs(&intent, &material));
    let publisher = scripted_publisher(config, runner);
    let issued = token();
    let verified = publisher
        .publish(&intent, &material, issued.token())
        .expect("verified");
    assert_eq!(verified.evidence().referrers().len(), 4);
}

#[test]
fn interrupted_upload_is_retryable_without_local_mutation() {
    let (_root, config, material, intent) = setup();
    let publisher = scripted_publisher(
        config,
        ScriptedRunner::with(vec![CommandOutput::failure("connection reset")]),
    );
    let issued = token();
    assert!(matches!(
        publisher.publish(&intent, &material, issued.token()),
        Err(PublisherError::CommandFailed)
    ));
    assert_eq!(intent.state(), PublicationState::Pending);
}

#[test]
fn rejects_malformed_layout_index_before_commands() {
    let (_root, config, material, intent) = setup();
    fs::write(material.layout.join("index.json"), b"not json").expect("rewrite");
    let publisher = scripted_publisher(config, ScriptedRunner::default());
    let issued = token();
    assert!(matches!(
        publisher.publish(&intent, &material, issued.token()),
        Err(PublisherError::MalformedLocalIndex)
    ));
}

#[test]
fn rejects_malformed_subject_index_before_commands() {
    let (_root, config, material, _intent) = setup();
    let bytes = b"not an OCI index";
    let expected = descriptor_for_bytes(bytes, OCI_INDEX_MEDIA_TYPE);
    let tag = local_reference_tag(expected.digest());
    fs::write(
        material.layout.join("index.json"),
        format!(
            r#"{{"manifests":[{{"mediaType":"{OCI_INDEX_MEDIA_TYPE}","digest":"{}","size":{},"annotations":{{"{OCI_REFERENCE_NAME_ANNOTATION}":"{tag}"}}}}]}}"#,
            expected.digest(),
            expected.size()
        ),
    )
    .expect("index");
    fs::write(
        material
            .layout
            .join("blobs/sha256")
            .join(expected.digest().as_str().trim_start_matches("sha256:")),
        bytes,
    )
    .expect("blob");
    let publisher = scripted_publisher(config, ScriptedRunner::default());
    let issued = token();
    assert!(matches!(
        publisher.publish(&intent(expected), &material, issued.token()),
        Err(PublisherError::MalformedLocalIndex)
    ));
}

#[test]
fn rejects_wrong_remote_digest() {
    let (_root, config, material, intent) = setup();
    let mut outputs = successful_outputs(&intent, &material);
    outputs[4] = CommandOutput::success(format!(
        r#"{{"mediaType":"{OCI_INDEX_MEDIA_TYPE}","digest":"{}","size":256}}"#,
        digest(E)
    ));
    let publisher = scripted_publisher(config, ScriptedRunner::with(outputs));
    let token = token();
    assert!(matches!(
        publisher.publish(&intent, &material, token.token()),
        Err(PublisherError::WrongRemoteDescriptor)
    ));
}

#[test]
fn rejects_missing_or_wrong_subject_referrer() {
    let (_root, config, material, intent) = setup();
    let mut missing = successful_outputs(&intent, &material);
    missing[6] = CommandOutput::success(r#"{"manifests":[]}"#);
    let publisher = scripted_publisher(config.clone(), ScriptedRunner::with(missing));
    let issued = token();
    assert!(matches!(
        publisher.publish(&intent, &material, issued.token()),
        Err(PublisherError::MissingOrDuplicateReferrer(_))
    ));
    let wrong_subject = successful_outputs_for_subject(&intent, &material, &digest(E), false);
    let publisher = scripted_publisher(config, ScriptedRunner::with(wrong_subject));
    let issued = token();
    assert!(matches!(
        publisher.publish(&intent, &material, issued.token()),
        Err(PublisherError::WrongReferrerSubject)
    ));
}

#[test]
fn classifies_expired_authentication_without_token_disclosure() {
    let (_root, config, material, intent) = setup();
    let runner = ScriptedRunner::with(vec![CommandOutput::failure("token expired: secret-token")]);
    let publisher = scripted_publisher(config, runner);
    let token = token();
    let error = publisher
        .publish(&intent, &material, token.token())
        .expect_err("auth error");
    assert!(matches!(error, PublisherError::AuthenticationFailed));
    assert!(!error.to_string().contains("secret-token"));
}

#[test]
fn duplicate_retry_is_safe() {
    let (_root, config, material, intent) = setup();
    let mut outputs = successful_outputs(&intent, &material);
    outputs.extend(successful_outputs(&intent, &material));
    let publisher = scripted_publisher(config, ScriptedRunner::with(outputs));
    let issued = token();
    let first = publisher
        .publish(&intent, &material, issued.token())
        .expect("first verification");
    let second = publisher
        .publish(&intent, &material, issued.token())
        .expect("retry verification");
    assert_eq!(first, second);
}

#[test]
fn rechecks_missing_content_without_republishing() {
    let (_root, config, material, intent) = setup();
    let outputs = successful_outputs(&intent, &material);
    let first = scripted_publisher(config.clone(), ScriptedRunner::with(outputs.clone()))
        .publish(&intent, &material, token().token())
        .expect("initial verification");
    let missing = intent
        .record_verified(first)
        .expect("verified")
        .approve()
        .expect("approved")
        .mark_missing()
        .expect("missing");
    let publisher = scripted_publisher(config, ScriptedRunner::with(outputs[4..].to_vec()));
    let verified = publisher
        .verify_existing(&missing, &material, token().token())
        .expect("reverified");
    assert_eq!(verified.manifest(), missing.expected_manifest());
}
