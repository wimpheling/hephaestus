use super::{OTHER_COMMIT, VALID_COMMIT, commit};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct Fixture {
    pub public_repository: Uuid,
    pub private_repository: Uuid,
    pub public_receive: Uuid,
    pub private_receive: Uuid,
    pub public_build: Uuid,
    pub second_public_build: Uuid,
    pub invalid_build: Uuid,
    pub private_build: Uuid,
    pub other_commit_build: Uuid,
    pub public_revision: Uuid,
    pub private_revision: Uuid,
    pub oversized_revision: Uuid,
    pub outsider_user: Uuid,
}

// Keep the disposable database fixture together so every foreign-key and RLS
// identity used by this schema test is visible in one setup path.
#[allow(clippy::too_many_lines)]
pub async fn seed_fixture(pool: &PgPool) -> Fixture {
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let private_project = Uuid::new_v4();
    let owner = Uuid::new_v4();
    let outsider_user = Uuid::new_v4();
    let public_repository = Uuid::new_v4();
    let private_repository = Uuid::new_v4();
    let public_receive = Uuid::new_v4();
    let private_receive = Uuid::new_v4();
    let public_build = Uuid::new_v4();
    let second_public_build = Uuid::new_v4();
    let invalid_build = Uuid::new_v4();
    let private_build = Uuid::new_v4();
    let other_commit_build = Uuid::new_v4();
    let public_revision = Uuid::new_v4();
    let private_revision = Uuid::new_v4();
    let oversized_revision = Uuid::new_v4();

    for (id, name) in [
        (owner, "UI schema owner"),
        (outsider_user, "UI schema outsider"),
    ] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(pool)
            .await
            .expect("user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'ui-schema-org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(owner)
    .execute(pool)
    .await
    .expect("organization owner");
    for (id, name) in [
        (project, "ui-schema-project"),
        (private_project, "ui-schema-private"),
    ] {
        sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(organization)
            .bind(name)
            .execute(pool)
            .await
            .expect("project");
    }
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project)
        .bind(owner)
        .execute(pool)
        .await
        .expect("project maintainer");
    for (id, project_id, name, public) in [
        (public_repository, project, "ui-schema-public", true),
        (
            private_repository,
            private_project,
            "ui-schema-private",
            false,
        ),
    ] {
        sqlx::query(
            "INSERT INTO repositories (id, project_id, name, is_public)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(project_id)
        .bind(name)
        .bind(public)
        .execute(pool)
        .await
        .expect("repository");
    }
    for (id, repository_id) in [
        (public_receive, public_repository),
        (private_receive, private_repository),
    ] {
        sqlx::query(
            "INSERT INTO git_receives
             (id, repository_id, principal, status, accepted_at)
             VALUES ($1, $2, 'ui-schema-test', 'accepted', now())",
        )
        .bind(id)
        .bind(repository_id)
        .execute(pool)
        .await
        .expect("receive");
    }
    for (id, repository_id, source_commit, receive_id) in [
        (
            public_build,
            public_repository,
            commit(VALID_COMMIT),
            public_receive,
        ),
        (
            invalid_build,
            public_repository,
            commit("c"),
            public_receive,
        ),
        (
            private_build,
            private_repository,
            commit(VALID_COMMIT),
            private_receive,
        ),
        (
            other_commit_build,
            public_repository,
            commit(OTHER_COMMIT),
            public_receive,
        ),
    ] {
        sqlx::query(
            "INSERT INTO build_requests
             (id, repository_id, source_commit, source_ref, origin_receive_id,
              build_definition_hash, state)
             VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'queued')",
        )
        .bind(id)
        .bind(repository_id)
        .bind(source_commit)
        .bind(receive_id)
        .bind([8_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("build request");
    }
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'queued')",
    )
    .bind(second_public_build)
    .bind(public_repository)
    .bind(commit(VALID_COMMIT))
    .bind(public_receive)
    .bind([9_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("second matching build request");
    Fixture {
        public_repository,
        private_repository,
        public_receive,
        private_receive,
        public_build,
        second_public_build,
        invalid_build,
        private_build,
        other_commit_build,
        public_revision,
        private_revision,
        oversized_revision,
        outsider_user,
    }
}
