use super::*;
use crate::{ReleaseId, UiInstallationGenerationId, UiInstallationId, ui::UiKey};
use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use std::fmt::Write as _;
use uuid::Uuid;

fn key(value: &str) -> UiKey {
    UiKey::parse(value).expect("valid UI key")
}

fn caller(value: &str) -> UiInstallationCallerKey {
    UiInstallationCallerKey::parse(value).expect("valid caller key")
}

#[test]
fn caller_key_is_bounded_and_printable() {
    assert!(UiInstallationCallerKey::parse("request-1").is_ok());
    assert!(UiInstallationCallerKey::parse("").is_err());
    assert!(UiInstallationCallerKey::parse(" request").is_ok());
    assert!(UiInstallationCallerKey::parse("request\n1").is_ok());
    assert!(UiInstallationCallerKey::parse("request\0").is_err());
    assert!(
        UiInstallationCallerKey::parse("a".repeat(MAX_UI_INSTALLATION_CALLER_KEY_BYTES)).is_ok()
    );
    assert!(
        UiInstallationCallerKey::parse("a".repeat(MAX_UI_INSTALLATION_CALLER_KEY_BYTES + 1))
            .is_err()
    );
}

#[test]
fn command_key_binds_actor_operation_and_caller_key() {
    let actor = Uuid::from_u128(1);
    let first = UiInstallationCommandIdentity::new(
        actor,
        UiInstallationOperation::Install,
        caller("request-1"),
    );
    assert_eq!(
        first.command_key(),
        UiInstallationCommandIdentity::new(
            actor,
            UiInstallationOperation::Install,
            caller("request-1"),
        )
        .command_key()
    );
    assert_ne!(
        first.command_key(),
        UiInstallationCommandIdentity::new(
            Uuid::from_u128(2),
            UiInstallationOperation::Install,
            caller("request-1"),
        )
        .command_key()
    );
    assert_ne!(
        first.command_key(),
        UiInstallationCommandIdentity::new(
            actor,
            UiInstallationOperation::Activate,
            caller("request-1"),
        )
        .command_key()
    );
    assert_ne!(
        first.command_key(),
        UiInstallationCommandIdentity::new(
            actor,
            UiInstallationOperation::Install,
            caller("request-2"),
        )
        .command_key()
    );
}

#[test]
fn changed_input_keeps_command_key_but_changes_input_digest() {
    let actor = Uuid::from_u128(1);
    let identity = UiInstallationCommandIdentity::new(
        actor,
        UiInstallationOperation::Install,
        caller("request-1"),
    );
    let same_identity = UiInstallationCommandIdentity::new(
        actor,
        UiInstallationOperation::Install,
        caller("request-1"),
    );
    let target = UiInstallationTarget::project(ProjectId::from_uuid(Uuid::from_u128(2)));
    let release = ReleaseId::from_uuid(Uuid::from_u128(3));
    let first = UiInstallationInputDigest::install(target, release, &key("assistant"));
    let changed = UiInstallationInputDigest::install(target, release, &key("other"));
    assert_eq!(identity.command_key(), same_identity.command_key());
    assert_ne!(first, changed);
}

#[test]
fn expected_organization_digest_is_opt_in_and_distinct() {
    let target = UiInstallationTarget::project(ProjectId::from_uuid(Uuid::from_u128(2)));
    let release = ReleaseId::from_uuid(Uuid::from_u128(3));
    let ui_key = key("assistant");
    let organization_a = OrganizationId::from_uuid(Uuid::from_u128(4));
    let organization_b = OrganizationId::from_uuid(Uuid::from_u128(5));
    let legacy = UiInstallationInputDigest::install(target, release, &ui_key);
    let none = UiInstallationInputDigest::install_with_expected_organization(
        target, release, &ui_key, None,
    );
    let acknowledged = UiInstallationInputDigest::install_with_expected_organization_and_git_ack(
        target, release, &ui_key, None, true,
    );
    let acknowledged_replay =
        UiInstallationInputDigest::install_with_expected_organization_and_git_ack(
            target, release, &ui_key, None, true,
        );
    let scoped_a = UiInstallationInputDigest::install_with_expected_organization(
        target,
        release,
        &ui_key,
        Some(organization_a),
    );
    let scoped_b = UiInstallationInputDigest::install_with_expected_organization(
        target,
        release,
        &ui_key,
        Some(organization_b),
    );
    assert_eq!(legacy, none);
    assert_ne!(none, acknowledged);
    assert_eq!(acknowledged, acknowledged_replay);
    assert_ne!(legacy, scoped_a);
    assert_ne!(scoped_a, scoped_b);
}

#[test]
fn compare_and_swap_and_operation_inputs_are_distinct() {
    let installation = UiInstallationId::from_uuid(Uuid::from_u128(1));
    let generation = UiInstallationGenerationId::from_uuid(Uuid::from_u128(2));
    let release = ReleaseId::from_uuid(Uuid::from_u128(3));
    let ui_key = key("assistant");
    assert_ne!(
        UiInstallationInputDigest::activate(installation, None, release, &ui_key),
        UiInstallationInputDigest::activate(installation, Some(generation), release, &ui_key,)
    );
    assert_ne!(
        UiInstallationInputDigest::disable(installation, Some(generation)),
        UiInstallationInputDigest::remove(installation, Some(generation)),
    );
    assert_ne!(
        UiInstallationInputDigest::activate(installation, Some(generation), release, &ui_key),
        UiInstallationInputDigest::rollback(installation, Some(generation), release, &ui_key),
    );
    assert_ne!(
        UiInstallationInputDigest::install(
            UiInstallationTarget::project(ProjectId::from_uuid(Uuid::from_u128(4))),
            release,
            &ui_key,
        ),
        UiInstallationInputDigest::install(
            UiInstallationTarget::repository(RepositoryId::from_uuid(Uuid::from_u128(4))),
            release,
            &ui_key,
        ),
    );
    assert_ne!(
        UiInstallationInputDigest::install(
            UiInstallationTarget::project(ProjectId::from_uuid(Uuid::from_u128(4))),
            release,
            &ui_key,
        ),
        UiInstallationInputDigest::install(
            UiInstallationTarget::organization(OrganizationId::from_uuid(Uuid::from_u128(4))),
            release,
            &ui_key,
        ),
    );
    assert_ne!(
        UiInstallationInputDigest::install(
            UiInstallationTarget::repository(RepositoryId::from_uuid(Uuid::from_u128(4))),
            release,
            &ui_key,
        ),
        UiInstallationInputDigest::install(
            UiInstallationTarget::organization(OrganizationId::from_uuid(Uuid::from_u128(4))),
            release,
            &ui_key,
        ),
    );
}

#[test]
fn global_target_round_trips_with_global_scope_and_same_uuid_isolated() {
    let target = UiInstallationTarget::organization(OrganizationId::from_uuid(Uuid::from_u128(4)));
    let json = serde_json::to_value(target).expect("serialize global target");
    assert_eq!(json["scope"], "global");
    let decoded: UiInstallationTarget =
        serde_json::from_value(json).expect("deserialize global target");
    assert_eq!(decoded, target);
    assert_ne!(
        UiInstallationInputDigest::install(
            target,
            ReleaseId::from_uuid(Uuid::from_u128(3)),
            &key("assistant"),
        ),
        UiInstallationInputDigest::install(
            UiInstallationTarget::project(ProjectId::from_uuid(Uuid::from_u128(4))),
            ReleaseId::from_uuid(Uuid::from_u128(3)),
            &key("assistant"),
        )
    );
}

#[test]
fn fresh_caller_key_reuses_input_digest_but_changes_command_key() {
    let first = UiInstallationCommandIdentity::new(
        Uuid::from_u128(1),
        UiInstallationOperation::Activate,
        caller("request-1"),
    );
    let second = UiInstallationCommandIdentity::new(
        Uuid::from_u128(1),
        UiInstallationOperation::Activate,
        caller("request-2"),
    );
    let input = UiInstallationInputDigest::activate(
        UiInstallationId::from_uuid(Uuid::from_u128(2)),
        None,
        ReleaseId::from_uuid(Uuid::from_u128(3)),
        &key("assistant"),
    );
    let repeated = UiInstallationInputDigest::activate(
        UiInstallationId::from_uuid(Uuid::from_u128(2)),
        None,
        ReleaseId::from_uuid(Uuid::from_u128(3)),
        &key("assistant"),
    );
    assert_eq!(input, repeated);
    assert_ne!(first.command_key(), second.command_key());
}

#[test]
fn canonical_v1_hash_vectors_are_stable() {
    let command = UiInstallationCommandIdentity::new(
        Uuid::from_u128(1),
        UiInstallationOperation::Install,
        caller("request-1"),
    )
    .command_key();
    assert_eq!(
        hex(command.as_bytes()),
        "f8c3340ccd9631ebd1f46fb3ef028baf4867df97057572326af43bf478f81a2c"
    );
    let digest = UiInstallationInputDigest::install(
        UiInstallationTarget::project(ProjectId::from_uuid(Uuid::from_u128(2))),
        ReleaseId::from_uuid(Uuid::from_u128(3)),
        &key("assistant"),
    );
    assert_eq!(
        hex(digest.as_bytes()),
        "7fa65774e07cca5430f05b76b3f47307bc6112d3b042c2368fe8d6dc7dc6b536"
    );
    let global_digest = UiInstallationInputDigest::install(
        UiInstallationTarget::organization(OrganizationId::from_uuid(Uuid::from_u128(4))),
        ReleaseId::from_uuid(Uuid::from_u128(3)),
        &key("assistant"),
    );
    assert_eq!(
        hex(global_digest.as_bytes()),
        "1d1ae21cbc5353f9924d426f624115c15b5703acb55d1a10dd90499a097435fe"
    );
}

fn hex(value: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in value {
        write!(&mut output, "{byte:02x}").expect("write into string");
    }
    output
}

#[test]
fn lifecycle_keeps_removed_terminal() {
    assert!(UiInstallationState::Enabled.can_transition_to(UiInstallationState::Enabled));
    assert!(UiInstallationState::Enabled.can_transition_to(UiInstallationState::Disabled));
    assert!(UiInstallationState::Disabled.can_transition_to(UiInstallationState::Disabled));
    assert!(UiInstallationState::Disabled.can_transition_to(UiInstallationState::Enabled));
    assert!(UiInstallationState::Disabled.can_transition_to(UiInstallationState::Removed));
    for state in [
        UiInstallationState::Enabled,
        UiInstallationState::Disabled,
        UiInstallationState::Removed,
    ] {
        assert!(!UiInstallationState::Removed.can_transition_to(state));
    }
}
