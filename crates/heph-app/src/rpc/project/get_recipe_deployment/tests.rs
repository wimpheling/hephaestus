//! Controlled model and transport tests; these do not prove PG role/provenance.

pub(in crate::rpc::project) mod authentication;

use super::{map_error, projection};
use crate::rpc::RpcError;
use forge_domain::ProjectId;
use recipe_application::{
    DeploymentAttemptId, DeploymentId, DeploymentIntent, DeploymentKey, DeploymentLifecycle,
    DeploymentSnapshot, DiagnosticCode, InstallProgress, RemovalProgress, ResourceProgress,
};
use rpc_proto::messages::hephaestus::project::v1::{
    RecipeActivationObservation, RecipeInstallProgress, RecipeProgressDiagnostic,
    RecipeRemovalProgress,
};
use std::collections::BTreeMap;
use uuid::Uuid;

fn snapshot() -> DeploymentSnapshot {
    let declaration = recipe_domain::parse_recipe(
        br#"
contract_version=1
recipe_id="inspection-model"
recipe_version="1.0.0"
[[inputs]]
name="capacity"
default=16777216
value_type={type="integer",minimum=16777216,maximum=1073741824}
[[resources]]
kind="volume"
name="data"
source={type="created",capacity_bytes={source="input",name="capacity"}}
"#,
    )
    .expect("controlled validated recipe");
    let resolved = declaration
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[])
        .expect("controlled pure resolution");
    let intent = DeploymentIntent::new(
        DeploymentId::from_uuid(Uuid::from_u128(1)).unwrap(),
        ProjectId::from_uuid(Uuid::from_u128(2)),
        DeploymentKey::parse("sqlite-data").unwrap(),
        &declaration,
        &resolved,
    )
    .expect("controlled immutable model, not PG lineage");
    let resources = intent
        .resources()
        .keys()
        .map(|name| {
            (
                name.clone(),
                ResourceProgress {
                    install: InstallProgress::RecoveryRequired,
                    removal: RemovalProgress::Retained,
                    version: 7,
                    active_attempt: Some(
                        DeploymentAttemptId::from_uuid(Uuid::from_u128(3)).unwrap(),
                    ),
                    diagnostic: Some(DiagnosticCode::ProviderOutcomeUnknown),
                },
            )
        })
        .collect();
    DeploymentSnapshot {
        intent,
        lifecycle: DeploymentLifecycle::RecoveryRequired,
        version: 9,
        resources,
    }
}

#[test]
fn projects_original_progress_without_private_intent_or_activation_inference() {
    let snapshot = snapshot();
    let value = projection::project(
        &snapshot,
        snapshot.intent.project_id().as_uuid(),
        snapshot.intent.id(),
    )
    .unwrap();
    assert_eq!(value.key, "sqlite-data");
    assert_eq!(value.version, 9);
    assert_eq!(
        value.activation.as_known(),
        Some(RecipeActivationObservation::Unobserved)
    );
    let child = &value.resources[0];
    assert_eq!(child.name, "data");
    assert_eq!(child.version, 7);
    assert!(child.recovery_required);
    assert_eq!(
        child.install.as_known(),
        Some(RecipeInstallProgress::RecoveryRequired)
    );
    assert_eq!(
        child.removal.as_known(),
        Some(RecipeRemovalProgress::Retained)
    );
    assert_eq!(
        child.diagnostic.as_known(),
        Some(RecipeProgressDiagnostic::ProviderOutcomeUnknown)
    );
    assert_eq!(
        child.active_attempt_id.as_option().unwrap().value,
        Uuid::from_u128(3).to_string()
    );
    assert!(child.revision_id.as_option().is_none());
    let wire = serde_json::to_string(&value).unwrap();
    for excluded in [
        "inspection-model",
        "capacity",
        "filesystem",
        "preparation",
        "runtime",
        "configuration",
        "authorization",
    ] {
        assert!(!wire.contains(excluded));
    }
    assert!(wire.len() < 65_536);
}

#[test]
fn rejects_foreign_project_wrong_original_and_incomplete_progress() {
    let mut snapshot = snapshot();
    assert!(matches!(
        projection::project(&snapshot, Uuid::from_u128(4), snapshot.intent.id()),
        Err(RpcError::PermissionDenied)
    ));
    assert!(matches!(
        projection::project(
            &snapshot,
            snapshot.intent.project_id().as_uuid(),
            DeploymentId::from_uuid(Uuid::from_u128(5)).unwrap()
        ),
        Err(RpcError::FailedPrecondition)
    ));
    snapshot.resources.clear();
    assert!(matches!(
        projection::project(
            &snapshot,
            snapshot.intent.project_id().as_uuid(),
            snapshot.intent.id()
        ),
        Err(RpcError::FailedPrecondition)
    ));
}

#[test]
fn installed_ready_and_removed_preserve_unobserved_activation() {
    let mut snapshot = snapshot();
    for lifecycle in [DeploymentLifecycle::Installed, DeploymentLifecycle::Removed] {
        snapshot.lifecycle = lifecycle;
        for progress in snapshot.resources.values_mut() {
            progress.install = InstallProgress::Ready;
            progress.active_attempt = None;
            progress.diagnostic = None;
        }
        let value = projection::project(
            &snapshot,
            snapshot.intent.project_id().as_uuid(),
            snapshot.intent.id(),
        )
        .unwrap();
        assert_eq!(
            value.activation.as_known(),
            Some(RecipeActivationObservation::Unobserved)
        );
        assert!(!value.resources[0].recovery_required);
        assert_eq!(
            value.resources[0].diagnostic.as_known(),
            Some(RecipeProgressDiagnostic::Unspecified)
        );
    }
}

#[test]
fn denied_unavailable_and_inconsistent_data_stay_distinct_errors() {
    use recipe_application::DeploymentError;
    assert_eq!(
        map_error(&DeploymentError::AuthorizationDenied),
        RpcError::PermissionDenied
    );
    assert_eq!(
        map_error(&DeploymentError::Unavailable),
        RpcError::Unavailable
    );
    assert_eq!(
        map_error(&DeploymentError::Repository),
        RpcError::Unavailable
    );
    assert_eq!(
        map_error(&DeploymentError::UnknownState),
        RpcError::FailedPrecondition
    );
}
