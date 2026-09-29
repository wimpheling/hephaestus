use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct Fixture {
    pub actor: Uuid,
    pub outsider: Uuid,
    pub project: Uuid,
    pub repository: Uuid,
    pub repository_two: Uuid,
    pub release: Uuid,
    pub second_release: Uuid,
    pub release_agent: Uuid,
    pub second_release_agent: Uuid,
    pub ui_key: &'static str,
    pub repository_ui_key: &'static str,
    pub gateway: Uuid,
    pub second_gateway: Uuid,
    pub gateway_revision: Uuid,
    pub second_gateway_revision: Uuid,
    pub installation: Uuid,
    pub generation: Uuid,
    pub repository_installation: Uuid,
    pub repository_generation: Uuid,
    pub repository_two_installation: Uuid,
    pub repository_two_generation: Uuid,
    pub binding_key: &'static str,
    pub command: [u8; 32],
}
