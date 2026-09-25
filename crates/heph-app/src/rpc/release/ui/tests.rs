use super::{
    TargetReceiptScope, UiInstallationCursorCodec, cursor_scope, expected_generation_id,
    install_error, parse_handoff_secret, parse_uuid_value,
};
use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
use uuid::Uuid;

#[test]
fn malformed_opaque_ids_fail_closed() {
    assert!(
        parse_uuid_value(&OpaqueId {
            value: String::from("not-a-uuid"),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        parse_uuid_value(&OpaqueId {
            value: Uuid::new_v4().to_string(),
            ..Default::default()
        })
        .is_ok()
    );
}

#[cfg(test)]
mod handoff_audit_tests {
    use super::parse_handoff_secret;

    #[test]
    fn malformed_secret_is_rejected_before_store_call() {
        assert!(parse_handoff_secret(&[0_u8; 31]).is_err());
    }
}

#[cfg(test)]
mod receipt_scope_tests {
    use super::TargetReceiptScope;
    use forge_domain::{OrganizationId, ProjectId, RepositoryId};
    use release_domain::UiInstallationTarget;
    use uuid::Uuid;

    #[test]
    fn owner_event_receipt_scopes_match_append_owner_event() {
        let organization = OrganizationId::from_uuid(Uuid::from_u128(1));
        let project = ProjectId::from_uuid(Uuid::from_u128(2));
        let repository = RepositoryId::from_uuid(Uuid::from_u128(3));
        assert_eq!(
            (
                UiInstallationTarget::Organization(organization).aggregate_type(),
                UiInstallationTarget::Organization(organization).primary_scope_kind()
            ),
            ("organization", "organization")
        );
        assert_eq!(
            (
                UiInstallationTarget::Project(project).aggregate_type(),
                UiInstallationTarget::Project(project).primary_scope_kind()
            ),
            ("project", "project")
        );
        assert_eq!(
            (
                UiInstallationTarget::Repository(repository).aggregate_type(),
                UiInstallationTarget::Repository(repository).primary_scope_kind()
            ),
            ("repository", "project")
        );
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::UiInstallationCursorCodec;
    use forge_domain::{OrganizationId, ProjectId};
    use identity_domain::UserId;
    use release_domain::{UiInstallationId, UiInstallationTarget};
    use uuid::Uuid;

    #[test]
    fn cursor_round_trip_and_scope_binding() {
        let codec = UiInstallationCursorCodec::new([7; 32]);
        let actor = UserId::from_uuid(Uuid::from_u128(1));
        let organization = OrganizationId::from_uuid(Uuid::from_u128(2));
        let project = ProjectId::from_uuid(Uuid::from_u128(3));
        let target = UiInstallationTarget::Project(project);
        let installation = UiInstallationId::from_uuid(Uuid::from_u128(4));
        let token = codec.encode(installation, actor, organization, target);
        assert_eq!(
            codec.decode(&token, actor, organization, target),
            Ok(installation)
        );
        assert_eq!(
            codec.decode(
                &token,
                UserId::from_uuid(Uuid::from_u128(5)),
                organization,
                target
            ),
            Err(crate::rpc::RpcError::InvalidArgument)
        );
        assert_eq!(
            codec.decode(
                &token,
                actor,
                organization,
                UiInstallationTarget::Organization(organization)
            ),
            Err(crate::rpc::RpcError::InvalidArgument)
        );
        let mut tampered = token.into_bytes();
        let last = tampered.last_mut().expect("encoded cursor");
        *last = if *last == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(tampered).expect("ASCII cursor");
        assert_eq!(
            codec.decode(&tampered, actor, organization, target),
            Err(crate::rpc::RpcError::InvalidArgument)
        );
    }

    #[test]
    fn cursor_tag_delimiter_does_not_change_fixed_boundary() {
        let codec = UiInstallationCursorCodec::new([7; 32]);
        let actor = UserId::from_uuid(Uuid::from_u128(1));
        let organization = OrganizationId::from_uuid(Uuid::from_u128(2));
        let project = ProjectId::from_uuid(Uuid::from_u128(3));
        let target = UiInstallationTarget::Project(project);
        let installation = (0..1_000_u128)
            .map(|value| UiInstallationId::from_uuid(Uuid::from_u128(value)))
            .find(|id| {
                let (kind, target_id) = super::cursor_scope(target);
                let payload = format!(
                    "v1|{actor}|{organization}|{kind}|{target_id}|{}|created_at-desc-id-desc",
                    id.as_uuid()
                );
                codec.tag(payload.as_bytes()).contains(&b'|')
            })
            .expect("test range includes an HMAC tag containing the delimiter");
        let token = codec.encode(installation, actor, organization, target);
        assert_eq!(
            codec.decode(&token, actor, organization, target),
            Ok(installation)
        );
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::{expected_generation_id, install_error};
    use crate::rpc::RpcError;
    use release_service::UiInstallationError;
    use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
    use uuid::Uuid;

    #[test]
    fn lifecycle_cas_is_required_and_uuid_validated() {
        assert!(expected_generation_id(None).is_err());
        assert!(expected_generation_id(Some(&OpaqueId::default())).is_err());
        assert!(
            expected_generation_id(Some(&OpaqueId {
                value: Uuid::new_v4().to_string(),
                ..Default::default()
            }))
            .is_ok()
        );
    }

    #[test]
    fn lifecycle_conflicts_are_failed_preconditions() {
        assert_eq!(
            RpcError::FailedPrecondition,
            install_error(UiInstallationError::GenerationConflict)
        );
        assert_eq!(
            RpcError::FailedPrecondition,
            install_error(UiInstallationError::InvalidTransition)
        );
        assert_eq!(
            RpcError::PermissionDenied,
            install_error(UiInstallationError::PermissionDenied)
        );
    }
}
