use super::super::load::{canonical_json_hash, resolve_captured_ui, verify_build_definition_hash};
use super::super::model::UiPublicationCandidates;
use super::common::{candidates, captured};
use release_domain::ReleaseAgentId;
use uuid::Uuid;

#[test]
fn valid_capture_resolves_exact_artifact_and_agent_ids() {
    let (ui_json, ui_hash, gateway_json, gateway_hash) = captured();
    let (artifacts, agents) = candidates();
    let result = resolve_captured_ui(
        Uuid::from_u128(3),
        ui_json,
        Some(ui_hash.as_slice()),
        true,
        Some(gateway_json),
        Some(gateway_hash.as_slice()),
        UiPublicationCandidates {
            repository_id: forge_domain::RepositoryId::from_uuid(Uuid::from_u128(4)),
            base_build_definition_hash: Some([11; 32]),
            static_artifacts: &artifacts,
            release_agents: &agents,
        },
    )
    .expect("valid captured UI");
    assert_eq!(result.source_manifest_revision_id, Uuid::from_u128(3));
    assert_eq!(
        result
            .static_uis
            .uis
            .iter()
            .find(|ui| ui.key.as_str() == "docs")
            .expect("static UI")
            .files[0]
            .artifact_id
            .as_uuid(),
        Uuid::from_u128(1)
    );
    assert_eq!(
        result
            .gateway_uis
            .uis
            .iter()
            .find(|ui| ui.key.as_str() == "assistant")
            .expect("managed UI")
            .managed_service
            .as_ref()
            .expect("managed binding")
            .release_agent_id,
        ReleaseAgentId::from_uuid(Uuid::from_u128(2))
    );
}

#[test]
fn hash_mismatch_and_missing_artifact_or_agent_are_redacted_failures() {
    let (ui_json, mut ui_hash, gateway_json, gateway_hash) = captured();
    let (artifacts, agents) = candidates();
    ui_hash[0] ^= 1;
    let error = resolve_captured_ui(
        Uuid::from_u128(5),
        ui_json.clone(),
        Some(ui_hash.as_slice()),
        true,
        Some(gateway_json.clone()),
        Some(gateway_hash.as_slice()),
        UiPublicationCandidates {
            repository_id: forge_domain::RepositoryId::from_uuid(Uuid::from_u128(6)),
            base_build_definition_hash: Some([11; 32]),
            static_artifacts: &artifacts,
            release_agents: &agents,
        },
    )
    .expect_err("hash mismatch");
    assert!(matches!(
        error,
        super::super::ReleaseServiceError::InvalidStoredData
    ));

    let error = resolve_captured_ui(
        Uuid::from_u128(7),
        ui_json.clone(),
        Some(
            canonical_json_hash(&serde_json::from_value(ui_json.clone()).expect("typed UI"))
                .expect("UI hash")
                .as_slice(),
        ),
        true,
        Some(gateway_json.clone()),
        Some(gateway_hash.as_slice()),
        UiPublicationCandidates {
            repository_id: forge_domain::RepositoryId::from_uuid(Uuid::from_u128(8)),
            base_build_definition_hash: Some([11; 32]),
            static_artifacts: &[],
            release_agents: &agents,
        },
    )
    .expect_err("missing artifact");
    assert!(matches!(
        error,
        super::super::ReleaseServiceError::InvalidStoredData
    ));

    let error = resolve_captured_ui(
        Uuid::from_u128(9),
        ui_json.clone(),
        Some(
            canonical_json_hash(&serde_json::from_value(ui_json).expect("typed UI"))
                .expect("UI hash")
                .as_slice(),
        ),
        true,
        Some(gateway_json),
        Some(gateway_hash.as_slice()),
        UiPublicationCandidates {
            repository_id: forge_domain::RepositoryId::from_uuid(Uuid::from_u128(10)),
            base_build_definition_hash: Some([11; 32]),
            static_artifacts: &artifacts,
            release_agents: &[],
        },
    )
    .expect_err("missing agent");
    assert!(matches!(
        error,
        super::super::ReleaseServiceError::InvalidStoredData
    ));
}

#[test]
fn derived_build_identity_must_match_the_stored_request_hash() {
    let base = [11; 32];
    let ui = [12; 32];
    let gateway = Some([13; 32]);
    let expected = agent_config::build_identity::ui_build_definition_hash(base, ui, gateway);
    verify_build_definition_hash(&expected, ui, gateway, base)
        .expect("matching derived build identity");
    let mut wrong = expected;
    wrong[0] ^= 1;
    assert!(matches!(
        verify_build_definition_hash(&wrong, ui, gateway, base),
        Err(super::super::ReleaseServiceError::InvalidStoredData)
    ));
}
