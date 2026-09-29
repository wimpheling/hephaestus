use uuid::Uuid;

// The shared fixture also feeds the gateway and navigation matrices; each
// resource test intentionally consumes only the identities relevant to it.
#[allow(dead_code)]
pub struct Fixture {
    pub actor: Uuid,
    pub outsider: Uuid,
    pub organization: Uuid,
    pub project: Uuid,
    pub source_project: Uuid,
    pub repository: Uuid,
    pub release: Uuid,
    pub release_agent: Uuid,
    pub other_organization: Uuid,
    pub parent_session: Uuid,
    pub outsider_parent_session: Uuid,
    pub installation: Uuid,
    pub other_installation: Uuid,
    pub generation: Uuid,
    pub other_generation: Uuid,
    pub global_installation: Uuid,
    pub global_generation: Uuid,
    pub repository_installation: Uuid,
    pub repository_generation: Uuid,
    pub managed_installation: Uuid,
    pub managed_generation: Uuid,
    pub managed_gateway: Uuid,
    pub managed_revision: Uuid,
    pub route: &'static str,
}

pub fn fixture_session_secret(actor: Uuid, seed: u8) -> [u8; 32] {
    let mut secret = [seed; 32];
    secret[..16].copy_from_slice(actor.as_bytes());
    secret[16..].fill(seed);
    secret
}

pub fn digest(seed: u8) -> Vec<u8> {
    let mut value = vec![seed; 32];
    value[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    value
}
