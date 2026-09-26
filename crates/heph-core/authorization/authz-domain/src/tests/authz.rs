//! Authorization model parsing tests.

use super::{ObjectType, Permission, Subject};
use runtime_types::AgentInstanceId;
use std::str::FromStr;

#[test]
fn validates_model_names() {
    assert_eq!(
        ObjectType::from_str("state_volume").expect("known type"),
        ObjectType::StateVolume
    );
    assert_eq!(
        ObjectType::from_str("repository_oci_image").expect("known type"),
        ObjectType::RepositoryOciImage
    );
    assert_eq!(
        ObjectType::from_str("gateway_revision").expect("known type"),
        ObjectType::GatewayRevision
    );
    assert!(ObjectType::from_str("unknown").is_err());
    assert_eq!(
        Permission::from_str("can_execute").expect("known permission"),
        Permission::CanExecute
    );
    assert_eq!(
        Permission::from_str("can_revoke").expect("known permission"),
        Permission::CanRevoke
    );
    assert_eq!(
        Permission::from_str("can_grant_agent_capability").expect("known permission"),
        Permission::CanGrantAgentCapability
    );
    assert_eq!(
        Permission::from_str("agent_update_ref").expect("known permission"),
        Permission::AgentUpdateRef
    );
    assert!(Permission::from_str("owner").is_err());
}

#[test]
fn agent_instance_subject_uses_canonical_model_name() {
    let id = AgentInstanceId::new();
    let subject = Subject::AgentInstance(id);
    assert_eq!(subject.object_type(), "agent_instance");
    assert_eq!(subject.id(), id.to_string());
}
