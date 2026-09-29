use super::{
    AgentFamilyId, AgentKey, ArtifactPath, BuildState, ContentHash, InstanceState, NetworkAccess,
    ParameterDeclaration, ParameterDiagnosticCode, ParameterDocument, ParameterName, ParameterType,
    ParameterValue, RefSelector, ReleaseCommandKey, ReleaseState, ReleaseVersion, RuntimePolicy,
    UpdateState, validate_attachment_project, validate_update_family,
};
use forge_domain::{GitRef, ProjectId};
use std::collections::BTreeMap;

#[test]
fn identifiers_and_bounded_values_round_trip() {
    let id = super::ReleaseId::new();
    let serialized = serde_json::to_string(&id).expect("identifier should serialize");
    assert_eq!(
        serde_json::from_str::<super::ReleaseId>(&serialized)
            .expect("identifier should deserialize"),
        id
    );
    assert_eq!(
        id.to_string()
            .parse::<super::ReleaseId>()
            .expect("identifier should parse"),
        id
    );
    assert!("invalid".parse::<super::ReleaseId>().is_err());
    assert!(ReleaseVersion::parse("v1.2.3+linux").is_ok());
    assert!(ReleaseVersion::parse("../escape").is_err());
    assert!(AgentKey::parse("reviewer-v2").is_ok());
    assert!(AgentKey::parse("Reviewer").is_err());
    assert!(ArtifactPath::parse("bin/reviewer").is_ok());
    assert!(ArtifactPath::parse("../bin/reviewer").is_err());
    assert!(ArtifactPath::parse(".git/config").is_err());
}

#[test]
fn ref_selectors_match_exactly_or_below_prefix() {
    let exact = RefSelector::parse("refs/heads/main").expect("exact selector should parse");
    let prefix = RefSelector::parse("refs/heads/release/*").expect("prefix selector should parse");
    let main = GitRef::parse("refs/heads/main").expect("ref should parse");
    let release = GitRef::parse("refs/heads/release/v1").expect("ref should parse");
    assert!(exact.matches(&main));
    assert!(!exact.matches(&release));
    assert!(prefix.matches(&release));
    assert!(!prefix.matches(&main));
    assert!(RefSelector::parse("../../main").is_err());
}

#[test]
fn lifecycle_transitions_enforce_commit_points_and_run_gate() {
    assert!(BuildState::Queued.can_transition_to(BuildState::Running));
    assert!(!BuildState::Succeeded.can_transition_to(BuildState::Running));
    assert!(ReleaseState::Draft.can_transition_to(ReleaseState::Published));
    assert!(!ReleaseState::Published.can_transition_to(ReleaseState::Draft));

    assert!(InstanceState::Active.run_gate_open());
    assert!(!InstanceState::Updating.run_gate_open());
    assert!(InstanceState::Updating.can_transition_to(InstanceState::PausedUnknownState));
    assert!(UpdateState::HookRunning.can_transition_to(UpdateState::HookCommitted));
    assert!(UpdateState::HookCommitted.can_transition_to(UpdateState::ActivationRecovery));
    assert!(!UpdateState::HookCommitted.can_transition_to(UpdateState::Rejected));
}

#[test]
fn family_and_project_boundaries_use_stable_ids_not_names() {
    let family = AgentFamilyId::new();
    assert!(validate_update_family(family, family).is_ok());
    assert!(validate_update_family(family, AgentFamilyId::new()).is_err());

    let project = ProjectId::new();
    assert!(validate_attachment_project(project, project).is_ok());
    assert!(validate_attachment_project(project, ProjectId::new()).is_err());
}

#[test]
fn effective_policy_can_only_restrict() {
    let release = RuntimePolicy {
        vcpus: 8,
        memory_mib: 8192,
        network: NetworkAccess::Egress,
    };
    let platform = RuntimePolicy {
        vcpus: 16,
        memory_mib: 16_384,
        network: NetworkAccess::BrokerOnly,
    };
    let selected = RuntimePolicy {
        vcpus: 4,
        memory_mib: 4096,
        network: NetworkAccess::BrokerOnly,
    };
    assert_eq!(
        RuntimePolicy::resolve(&release, &selected, &platform)
            .expect("selection should fit both ceilings"),
        selected
    );
    let too_broad = RuntimePolicy {
        network: NetworkAccess::Egress,
        ..selected
    };
    assert!(RuntimePolicy::resolve(&release, &too_broad, &platform).is_err());
}

#[test]
fn typed_parameters_are_complete_and_deterministic() {
    let count = ParameterName::parse("count").expect("name should validate");
    let mode = ParameterName::parse("mode").expect("name should validate");
    let declarations = vec![
        ParameterDeclaration {
            name: count.clone(),
            value_type: ParameterType::Integer {
                minimum: 1,
                maximum: 10,
            },
            required: true,
            default: None,
            sensitive: false,
        },
        ParameterDeclaration {
            name: mode.clone(),
            value_type: ParameterType::Enum {
                values: vec![String::from("fast"), String::from("safe")],
            },
            required: false,
            default: Some(ParameterValue::String(String::from("safe"))),
            sensitive: false,
        },
    ];
    let values = BTreeMap::from([(count.clone(), ParameterValue::Integer(3))]);
    let first =
        ParameterDocument::resolve(&declarations, &values).expect("parameters should resolve");
    let repeated_values = BTreeMap::from([(count, ParameterValue::Integer(3))]);
    let second = ParameterDocument::resolve(&declarations, &repeated_values)
        .expect("parameters should resolve again");
    assert_eq!(first.hash(), second.hash());
    assert_eq!(
        first.values().get(&mode),
        Some(&ParameterValue::String(String::from("safe")))
    );

    let diagnostics = ParameterDocument::resolve(&declarations, &BTreeMap::new())
        .expect_err("required value should be diagnosed");
    assert_eq!(
        diagnostics[0].code,
        ParameterDiagnosticCode::RequiredMissing
    );
}

#[test]
fn release_update_parameter_diagnostics_are_stable_for_schema_changes() {
    let legacy = ParameterName::parse("legacy").expect("name should validate");
    let replacement = ParameterName::parse("replacement").expect("name should validate");
    let provided = BTreeMap::from([(
        legacy.clone(),
        ParameterValue::String(String::from("value")),
    )]);

    let removed = ParameterDocument::resolve(&[], &provided)
        .expect_err("removed parameter should be rejected");
    assert_eq!(
        removed,
        vec![super::ParameterDiagnostic {
            code: ParameterDiagnosticCode::UnknownParameter,
            parameter: Some(legacy.clone()),
        }]
    );

    let renamed_declarations = vec![ParameterDeclaration {
        name: replacement.clone(),
        value_type: ParameterType::String {
            minimum_length: 1,
            maximum_length: 32,
        },
        required: true,
        default: None,
        sensitive: false,
    }];
    let renamed = ParameterDocument::resolve(&renamed_declarations, &provided)
        .expect_err("renamed parameter should produce complete diagnostics");
    assert_eq!(
        renamed,
        vec![
            super::ParameterDiagnostic {
                code: ParameterDiagnosticCode::UnknownParameter,
                parameter: Some(legacy.clone()),
            },
            super::ParameterDiagnostic {
                code: ParameterDiagnosticCode::RequiredMissing,
                parameter: Some(replacement.clone()),
            },
        ]
    );

    let newly_required = ParameterDocument::resolve(&renamed_declarations, &BTreeMap::new())
        .expect_err("newly required parameter should be rejected");
    assert_eq!(
        newly_required,
        vec![super::ParameterDiagnostic {
            code: ParameterDiagnosticCode::RequiredMissing,
            parameter: Some(replacement),
        }]
    );

    let type_changed = ParameterDocument::resolve(
        &[ParameterDeclaration {
            name: legacy.clone(),
            value_type: ParameterType::Boolean,
            required: true,
            default: None,
            sensitive: false,
        }],
        &provided,
    )
    .expect_err("type-changed value should be rejected without coercion");
    assert_eq!(
        type_changed,
        vec![super::ParameterDiagnostic {
            code: ParameterDiagnosticCode::InvalidValue,
            parameter: Some(legacy),
        }]
    );
}

#[test]
fn deterministic_hashes_and_command_keys_are_unambiguous() {
    assert_eq!(
        ContentHash::digest(b"config"),
        ContentHash::digest(b"config")
    );
    assert_ne!(
        ContentHash::digest(b"config"),
        ContentHash::digest(b"artifact")
    );

    let first = ReleaseCommandKey::derive("build", &[b"ab", b"c"]);
    assert_eq!(first, ReleaseCommandKey::derive("build", &[b"ab", b"c"]));
    assert_ne!(first, ReleaseCommandKey::derive("build", &[b"a", b"bc"]));
    assert_ne!(first, ReleaseCommandKey::derive("release", &[b"ab", b"c"]));
}
