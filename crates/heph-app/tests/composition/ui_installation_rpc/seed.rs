use identity_domain::{AuthenticatedIdentity, BrowserSessionSid, RequestId, UserId};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    ui_installation_seed_base as seed_base, ui_installation_seed_releases as seed_releases,
    ui_installation_seed_sessions as seed_sessions,
};

const ISSUER: &str = "https://ui-rpc-test.invalid";

#[derive(Clone, Copy)]
pub(super) struct Fixture {
    pub(super) user_id: Uuid,
    pub(super) organization_id: Uuid,
    pub(super) foreign_organization_id: Uuid,
    pub(super) project_id: Uuid,
    pub(super) repository_id: Uuid,
    pub(super) release_id: Uuid,
    pub(super) global_release_id: Uuid,
    pub(super) repository_release_id: Uuid,
    pub(super) parent_session_id: Uuid,
    pub(super) sid: BrowserSessionSid,
    pub(super) second_user_id: Uuid,
    pub(super) second_sid: BrowserSessionSid,
}

pub(crate) struct SeedData {
    pub(crate) user_id: Uuid,
    pub(crate) organization_id: Uuid,
    pub(crate) foreign_organization_id: Uuid,
    pub(crate) project_id: Uuid,
    pub(crate) repository_id: Uuid,
    pub(crate) second_user_id: Uuid,
    pub(crate) receive_id: Uuid,
    pub(crate) build_id: Uuid,
    pub(crate) source_revision_id: Uuid,
    pub(crate) release_id: Uuid,
    pub(crate) global_release_id: Uuid,
    pub(crate) repository_release_id: Uuid,
    pub(crate) artifact_id: Uuid,
    pub(crate) global_artifact_id: Uuid,
    pub(crate) repository_artifact_id: Uuid,
    pub(crate) parent_session_id: Uuid,
    pub(crate) second_parent_session_id: Uuid,
    pub(crate) sid: BrowserSessionSid,
    pub(crate) second_sid: BrowserSessionSid,
    pub(crate) issuer: String,
    pub(crate) identity: AuthenticatedIdentity,
    pub(crate) issued_at: OffsetDateTime,
    pub(crate) commit: String,
}

impl SeedData {
    fn new() -> Self {
        let user_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let foreign_organization_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let repository_id = Uuid::new_v4();
        let second_user_id = Uuid::new_v4();
        let receive_id = Uuid::new_v4();
        let build_id = Uuid::new_v4();
        let source_revision_id = Uuid::new_v4();
        let release_id = Uuid::new_v4();
        let global_release_id = Uuid::new_v4();
        let repository_release_id = Uuid::new_v4();
        let artifact_id = Uuid::new_v4();
        let global_artifact_id = Uuid::new_v4();
        let repository_artifact_id = Uuid::new_v4();
        let parent_session_id = Uuid::new_v4();
        let second_parent_session_id = Uuid::new_v4();
        let sid = BrowserSessionSid::new();
        let second_sid = BrowserSessionSid::new();
        let issuer = String::from(ISSUER);
        let subject = String::from("ui-rpc-owner");
        let identity = AuthenticatedIdentity::new(
            UserId::from_uuid(user_id),
            issuer.clone(),
            subject,
            serde_json::json!({"email_verified": true}),
            RequestId::new(),
        );
        Self {
            user_id,
            organization_id,
            foreign_organization_id,
            project_id,
            repository_id,
            second_user_id,
            receive_id,
            build_id,
            source_revision_id,
            release_id,
            global_release_id,
            repository_release_id,
            artifact_id,
            global_artifact_id,
            repository_artifact_id,
            parent_session_id,
            second_parent_session_id,
            sid,
            second_sid,
            issuer,
            identity,
            issued_at: OffsetDateTime::now_utc(),
            commit: "b".repeat(40),
        }
    }
}

// Seed phases retain the original insertion order while keeping each concern small.
pub(crate) async fn seed_fixture(pool: &PgPool) -> Fixture {
    let data = SeedData::new();
    seed_base::seed_base_rows(pool, &data).await;
    seed_releases::seed_release_rows(pool, &data).await;
    seed_sessions::seed_session_rows(pool, &data).await;
    Fixture {
        user_id: data.user_id,
        organization_id: data.organization_id,
        foreign_organization_id: data.foreign_organization_id,
        project_id: data.project_id,
        repository_id: data.repository_id,
        release_id: data.release_id,
        global_release_id: data.global_release_id,
        repository_release_id: data.repository_release_id,
        parent_session_id: data.parent_session_id,
        sid: data.sid,
        second_user_id: data.second_user_id,
        second_sid: data.second_sid,
    }
}
