use super::*;

#[derive(Clone, Copy)]
pub struct Fixture {
    pub actor: Uuid,
    pub outsider: Uuid,
    pub organization: Uuid,
    pub project: Uuid,
    pub source_project: Uuid,
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
    pub no_git_repository_installation: Uuid,
    pub no_git_repository_generation: Uuid,
    pub write_repository_installation: Uuid,
    pub write_repository_generation: Uuid,
    pub managed_installation: Uuid,
    pub managed_generation: Uuid,
    pub managed_gateway: Uuid,
    pub managed_revision: Uuid,
    pub route: &'static str,
}

#[derive(Clone, Copy)]
pub struct FixtureSeedIds {
    pub actor: Uuid,
    pub outsider: Uuid,
    pub organization: Uuid,
    pub other_organization: Uuid,
    pub project: Uuid,
    pub source_project: Uuid,
    pub repository: Uuid,
    pub receive: Uuid,
    pub build: Uuid,
    pub source_revision: Uuid,
    pub release: Uuid,
    pub artifact: Uuid,
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
    pub no_git_repository_installation: Uuid,
    pub no_git_repository_generation: Uuid,
    pub write_repository_installation: Uuid,
    pub write_repository_generation: Uuid,
    pub managed_installation: Uuid,
    pub managed_generation: Uuid,
    pub managed_gateway: Uuid,
    pub managed_revision: Uuid,
    pub release_agent: Uuid,
    pub agent_family: Uuid,
    pub request_id: Uuid,
}

impl FixtureSeedIds {
    pub fn new() -> Self {
        Self {
            actor: Uuid::new_v4(),
            outsider: Uuid::new_v4(),
            organization: Uuid::new_v4(),
            other_organization: Uuid::new_v4(),
            project: Uuid::new_v4(),
            source_project: Uuid::new_v4(),
            repository: Uuid::new_v4(),
            receive: Uuid::new_v4(),
            build: Uuid::new_v4(),
            source_revision: Uuid::new_v4(),
            release: Uuid::new_v4(),
            artifact: Uuid::new_v4(),
            parent_session: Uuid::new_v4(),
            outsider_parent_session: Uuid::new_v4(),
            installation: Uuid::new_v4(),
            other_installation: Uuid::new_v4(),
            generation: Uuid::new_v4(),
            other_generation: Uuid::new_v4(),
            global_installation: Uuid::new_v4(),
            global_generation: Uuid::new_v4(),
            repository_installation: Uuid::new_v4(),
            repository_generation: Uuid::new_v4(),
            no_git_repository_installation: Uuid::new_v4(),
            no_git_repository_generation: Uuid::new_v4(),
            write_repository_installation: Uuid::new_v4(),
            write_repository_generation: Uuid::new_v4(),
            managed_installation: Uuid::new_v4(),
            managed_generation: Uuid::new_v4(),
            managed_gateway: Uuid::new_v4(),
            managed_revision: Uuid::new_v4(),
            release_agent: Uuid::new_v4(),
            agent_family: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
        }
    }

    pub const fn fixture(self) -> Fixture {
        Fixture {
            actor: self.actor,
            outsider: self.outsider,
            organization: self.organization,
            project: self.project,
            source_project: self.source_project,
            release: self.release,
            release_agent: self.release_agent,
            other_organization: self.other_organization,
            parent_session: self.parent_session,
            outsider_parent_session: self.outsider_parent_session,
            installation: self.installation,
            other_installation: self.other_installation,
            generation: self.generation,
            other_generation: self.other_generation,
            global_installation: self.global_installation,
            global_generation: self.global_generation,
            repository_installation: self.repository_installation,
            repository_generation: self.repository_generation,
            no_git_repository_installation: self.no_git_repository_installation,
            no_git_repository_generation: self.no_git_repository_generation,
            write_repository_installation: self.write_repository_installation,
            write_repository_generation: self.write_repository_generation,
            managed_installation: self.managed_installation,
            managed_generation: self.managed_generation,
            managed_gateway: self.managed_gateway,
            managed_revision: self.managed_revision,
            route: "schema-ui",
        }
    }
}

pub async fn insert_canonical_session(
    worker: &PgPool,
    session_id: Uuid,
    user_id: Uuid,
    request_id: Uuid,
    expiry_hours: i64,
) {
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + ($7::int * interval '1 hour'))",
    )
    .bind(session_id)
    .bind(digest(200))
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(digest(201))
    .bind(user_id)
    .bind(expiry_hours)
    .execute(worker)
    .await
    .expect("seed canonical human browser session");
}

pub fn digest(seed: u8) -> Vec<u8> {
    let mut value = vec![seed; 32];
    value[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    value
}

/// Derive a stable test secret from the fixture identity while keeping it
/// unique across fixture runs that share a real `PostgreSQL` database.
pub fn test_secret(actor: Uuid, seed: u8) -> [u8; 32] {
    let mut secret = [seed; 32];
    secret[..16].copy_from_slice(actor.as_bytes());
    secret[16..].fill(seed);
    secret
}

pub fn scoped_secret(actor: Uuid, mut secret: [u8; 32]) -> [u8; 32] {
    secret[..16].copy_from_slice(actor.as_bytes());
    secret
}
