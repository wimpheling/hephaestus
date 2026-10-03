use super::{AgentFamilyId, AgentKey, ArtifactPath, NetworkAccess, ReleaseAgent, RuntimePolicy};
use volume_domain::VolumeAccessMode;

#[test]
fn release_catalog_projects_legacy_state_without_mutating_identity_or_authored_slots() {
    let release = ReleaseAgent {
        id: super::ReleaseAgentId::new(),
        release_id: super::ReleaseId::new(),
        family_id: AgentFamilyId::new(),
        agent_key: AgentKey::parse("sqlite").expect("key"),
        display_name: "SQLite application".to_owned(),
        executable: ArtifactPath::parse("bin/sqlite-app").expect("executable"),
        arguments: Vec::new(),
        working_directory: ArtifactPath::parse("bin").expect("directory"),
        image_reference: "registry.example/app@sha256:pinned".to_owned(),
        requires_state: true,
        volume_slots: Vec::new(),
        policy_ceiling: RuntimePolicy {
            vcpus: 1,
            memory_mib: 128,
            network: NetworkAccess::Disabled,
        },
        parameters: Vec::new(),
        update_hook: None,
    };
    let before = release.clone();
    let catalog = release.effective_volume_slots().expect("legacy catalog");
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].slot().as_str(), "state");
    assert_eq!(catalog[0].access_mode(), VolumeAccessMode::ReadWrite);
    assert!(catalog[0].required());
    assert_eq!(release, before);
}
