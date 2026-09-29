// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
/// Builds a distinct cooking-agent release through the ordinary
/// Git/build/publish path. The guest keeps its declared model rule but sends
/// that request to the relay origin, so the host must deny the mismatch.
pub(crate) async fn build_and_publish_adversarial_agent(
    context: &CookingBuildContext<'_>,
    canonical: &PublishedCookingRepository,
) -> Result<PublishedCookingRepository, BuildError> {
    let source = context
        .root
        .join(format!("cooking-adversarial-agent-{}", Uuid::new_v4()));
    copy_source_tree(&canonical.source_path, &source)?;
    let source_file = source.join("cooking_agent.py");
    let source_code = fs::read_to_string(&source_file)?;
    let canonical_call = "destination = 'api.model.example' if model else 'relay.cooking.example'";
    let adversarial_call = "destination = 'relay.cooking.example'";
    let mutated = source_code.replacen(canonical_call, adversarial_call, 1);
    if mutated == source_code {
        return Err(invalid_state(
            "canonical cooking agent broker call changed unexpectedly",
        ));
    }
    fs::write(source_file, mutated)?;
    build_one(
        context,
        "cooking-agent-adversarial",
        source,
        "Cooking agent adversarial destination probe",
        true,
    )
    .await
}

/// Builds the canonical cooking agent after applying the checked-in guest
/// crash transformer. The transformed source is pushed and observed through
/// the same Git, build, and release workers as every ordinary fixture build.
pub(crate) async fn build_and_publish_guest_crash_agent(
    context: &CookingBuildContext<'_>,
    canonical: &PublishedCookingRepository,
) -> Result<PublishedCookingRepository, BuildError> {
    let source = context
        .root
        .join(format!("cooking-agent-guest-crash-{}", Uuid::new_v4()));
    copy_source_tree(&canonical.source_path, &source)?;
    let transformer = context.source_root.join("tests/guest_crash.py");
    let runtime_probe = context.source_root.join("tests/guest_confinement.py");
    let descriptor_staging = tempfile::Builder::new()
        .prefix(".guest-crash-probe-")
        .tempdir_in(context.root)?;
    let descriptor_root = descriptor_staging.path();
    let mut candidate_arguments = Vec::new();
    for (label, value) in [
        ("inbound", super::super::cooking::INBOUND_SENTINEL),
        ("model", super::super::cooking::MODEL_SENTINEL),
        ("relay", super::super::cooking::RELAY_SENTINEL),
        (
            "inbound_rotated",
            super::super::cooking::INBOUND_ROTATED_SENTINEL,
        ),
        (
            "model_rotated",
            super::super::cooking::MODEL_ROTATED_SENTINEL,
        ),
        (
            "relay_rotated",
            super::super::cooking::RELAY_ROTATED_SENTINEL,
        ),
    ] {
        let candidate = descriptor_root.join(label);
        fs::write(&candidate, value.as_bytes())?;
        candidate_arguments.push(format!("{label}={}", candidate.display()));
    }
    let descriptor = descriptor_root.join("descriptors.json");
    let mut generator = Command::new("python3");
    generator
        .arg(context.source_root.join("tests/guest_confinement_probe.py"))
        .arg("--output")
        .arg(&descriptor);
    for candidate in &candidate_arguments {
        generator.arg("--candidate").arg(candidate);
    }
    let generated = generator.status().await?;
    if !generated.success() {
        return Err(invalid_state(
            "guest confinement descriptor generation failed",
        ));
    }
    let status = Command::new("python3")
        .arg(&transformer)
        .arg(source.join("cooking_agent.py"))
        .arg(source.join("cooking_agent.py"))
        .arg("--runtime-source")
        .arg(&runtime_probe)
        .arg("--descriptor-source")
        .arg(&descriptor)
        .status()
        .await?;
    if !status.success() {
        return Err(invalid_state("guest crash source transformation failed"));
    }
    drop(descriptor_staging);
    build_one(
        context,
        "cooking-agent-guest-crash",
        source,
        "Cooking agent deterministic guest crash probe",
        true,
    )
    .await
}

/// Builds a release whose handler targets an undeclared mailbox slot.
///
/// It reuses the canonical gateway repository so release-family and route
/// identity remain the same while installation exercises a real new release.
pub(crate) async fn build_and_publish_adversarial_gateway(
    context: &CookingBuildContext<'_>,
    canonical: &PublishedCookingRepository,
) -> Result<PublishedCookingRepository, BuildError> {
    let source = context
        .root
        .join(format!("cooking-adversarial-gateway-{}", Uuid::new_v4()));
    copy_source_tree(&canonical.source_path, &source)?;
    initialize_git(&source, "Cooking gateway adversarial foreign-slot probe").await?;
    let remote = format!(
        "http://{}/{}",
        context.running.http_addr(),
        canonical.repository_id
    );
    git(&source, &["remote", "add", "origin", &remote]).await?;
    // This repository already contains the canonical release source.  Base
    // the adversarial commit on that remote head so the ordinary Git receive
    // fast-forward check remains enabled.
    authenticated_git(
        &source,
        context.identity.git_token,
        &["fetch", "origin", "refs/heads/main"],
    )
    .await?;
    git(&source, &["reset", "--hard", "FETCH_HEAD"]).await?;
    let source_path = source.join("src/main.rs");
    let source_code = fs::read_to_string(&source_path)?;
    let canonical_slot = "const COOKING_REQUESTS_SLOT: &str = \"cooking_requests\";";
    let foreign_slot =
        format!("const COOKING_REQUESTS_SLOT: &str = \"{FOREIGN_PUBLICATION_SLOT}\";");
    let adversarial_code = source_code.replace(canonical_slot, &foreign_slot);
    if adversarial_code == source_code {
        return Err(invalid_state(
            "canonical cooking gateway slot declaration changed unexpectedly",
        ));
    }
    fs::write(&source_path, adversarial_code)?;
    build_one_in_repository(
        context,
        "cooking-gateway-adversarial",
        source,
        "Cooking gateway adversarial foreign-slot probe",
        canonical.repository_id,
        true,
        false,
    )
    .await
}
