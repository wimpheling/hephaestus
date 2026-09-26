use super::*;

pub struct RecordingAuthorizer {
    pub calls: Mutex<Vec<GitOperation>>,
    pub identity: AuthenticatedIdentity,
}

pub struct TestIdentityProvider {
    pub identity: AuthenticatedIdentity,
}

#[async_trait]
impl GitAuthenticator for TestIdentityProvider {
    async fn authenticate(
        &self,
        _credential: Option<&str>,
        request_id: RequestId,
    ) -> Result<Principal, AuthenticationError> {
        let mut identity = self.identity.clone();
        identity.request_id = request_id;
        Ok(Principal::human(identity))
    }
}

#[tokio::test]
#[serial]
async fn postgres_authorizer_allows_reads_and_rejects_push_without_write() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Phase 3 migrations");
    let owner = UserId::new();
    let member = UserId::new();
    let organization = OrganizationId::new();
    let project = forge_domain::ProjectId::new();
    let repository = forge_domain::RepositoryId::new();
    for (user, name) in [(owner, "owner"), (member, "member")] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(user.as_uuid())
            .bind(name)
            .execute(&pool)
            .await
            .expect("user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'git-authz')")
        .bind(organization.as_uuid())
        .execute(&pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner'), ($1, $3, 'member')",
    )
    .bind(organization.as_uuid())
    .bind(owner.as_uuid())
    .bind(member.as_uuid())
    .execute(&pool)
    .await
    .expect("memberships");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
         VALUES ($1, $2, 'git-authz')",
    )
    .bind(project.as_uuid())
    .bind(organization.as_uuid())
    .execute(&pool)
    .await
    .expect("project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.as_uuid())
        .bind(owner.as_uuid())
        .execute(&pool)
        .await
        .expect("maintainer");
    sqlx::query(
        "INSERT INTO repositories
         (id, project_id, name, default_branch)
         VALUES ($1, $2, 'git-authz', 'refs/heads/main')",
    )
    .bind(repository.as_uuid())
    .bind(project.as_uuid())
    .execute(&pool)
    .await
    .expect("repository");

    let member_identity = AuthenticatedIdentity::new(
        member,
        "https://issuer.example",
        "member",
        json!({}),
        RequestId::new(),
    );
    let member_authorizer = PostgresGitAuthorizer::new(Arc::new(
        authz_postgres::PostgresGitAuthorizer::new(pool.clone()),
    ));
    for operation in [GitOperation::Clone, GitOperation::Fetch] {
        member_authorizer
            .authorize(&AuthorizationRequest {
                repository_id: repository,
                operation,
                principal: Principal::human(member_identity.clone()),
            })
            .await
            .expect("organization member may read Git");
    }
    assert!(
        member_authorizer
            .authorize(&AuthorizationRequest {
                repository_id: repository,
                operation: GitOperation::Push,
                principal: Principal::human(member_identity),
            })
            .await
            .is_err()
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE actor_id = $1 AND object_id = $2",
    )
    .bind(member.as_uuid())
    .bind(repository.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("authorization audits");
    assert_eq!(audits, 3);
}

#[async_trait]
impl GitAuthorizer for RecordingAuthorizer {
    async fn authorize(&self, request: &AuthorizationRequest) -> Result<(), AuthorizationError> {
        self.calls.lock().await.push(request.operation);
        Ok(())
    }
}
