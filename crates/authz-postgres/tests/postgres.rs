//! Opt-in real-PostgreSQL tests for Mélange evaluation and RLS.

use authz_domain::{AuthorizationDecision, AuthzError, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, begin_actor_transaction};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

#[derive(Clone, Copy)]
struct Fixture {
    owner: UserId,
    admin: UserId,
    maintainer: UserId,
    member: UserId,
    outsider: UserId,
    revoked: UserId,
    organization: Uuid,
    project: Uuid,
    consuming_project: Uuid,
    private_repository: Uuid,
    public_repository: Uuid,
    consuming_repository: Uuid,
    build: Uuid,
    release: Uuid,
    artifact: Uuid,
    release_agent: Uuid,
    instance: Uuid,
    attachment: Uuid,
    update: Uuid,
    run: Uuid,
    volume: Uuid,
}

#[derive(Clone, Copy)]
struct ExpectedCheck {
    subject: UserId,
    permission: Permission,
    object_type: ObjectType,
    object_id: Uuid,
    allowed: bool,
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn generated_permissions_and_rls_enforce_the_same_perimeter() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply Phase 3 migrations");
    let fixture = seed(&pool).await;
    let authorizer = PostgresMelangeAuthorizer;
    let mut contextless = pool.begin().await.expect("contextless transaction");
    assert!(matches!(
        authorizer
            .check(
                &mut contextless,
                Subject::User(fixture.owner),
                Permission::CanRead,
                ObjectRef::new(ObjectType::Repository, fixture.private_repository),
            )
            .await,
        Err(AuthzError::MissingActorContext)
    ));
    contextless
        .rollback()
        .await
        .expect("rollback contextless transaction");

    for check in parity_checks(&fixture) {
        let check_identity = identity(check.subject);
        let mut transaction = begin_actor_transaction(&pool, &check_identity)
            .await
            .expect("fixture actor transaction");
        let decision = authorizer
            .check(
                &mut transaction,
                Subject::User(check.subject),
                check.permission,
                ObjectRef::new(check.object_type, check.object_id),
            )
            .await
            .expect("Mélange fixture permission check");
        assert_eq!(
            decision.is_allowed(),
            check.allowed,
            "{} {}:{} for {}",
            check.permission.as_str(),
            check.object_type.as_str(),
            check.object_id,
            check.subject
        );
        transaction
            .rollback()
            .await
            .expect("rollback fixture actor transaction");
    }

    let maintainer_identity = identity(fixture.maintainer);
    let mut owner_tx = begin_actor_transaction(&pool, &maintainer_identity)
        .await
        .expect("maintainer actor transaction");
    for (permission, object_type, object_id) in [
        (
            Permission::CanWrite,
            ObjectType::Repository,
            fixture.private_repository,
        ),
        (
            Permission::CanExecute,
            ObjectType::AgentInstance,
            fixture.instance,
        ),
        (Permission::CanCancel, ObjectType::Run, fixture.run),
        (
            Permission::CanAttach,
            ObjectType::StateVolume,
            fixture.volume,
        ),
        (
            Permission::CanRestore,
            ObjectType::StateVolume,
            fixture.volume,
        ),
    ] {
        let decision = authorizer
            .check(
                &mut owner_tx,
                Subject::User(fixture.maintainer),
                permission,
                ObjectRef::new(object_type, object_id),
            )
            .await
            .expect("generated permission check");
        assert_eq!(decision, AuthorizationDecision::Allow);
    }
    owner_tx.rollback().await.expect("rollback owner checks");
    let owner_identity = identity(fixture.owner);
    let mut owner_tx = begin_actor_transaction(&pool, &owner_identity)
        .await
        .expect("owner actor transaction");
    assert_eq!(
        authorizer
            .check(
                &mut owner_tx,
                Subject::User(fixture.owner),
                Permission::CanDelete,
                ObjectRef::new(ObjectType::Repository, fixture.private_repository),
            )
            .await
            .expect("owner delete permission"),
        AuthorizationDecision::Allow
    );
    assert_eq!(
        authorizer
            .check(
                &mut owner_tx,
                Subject::User(fixture.owner),
                Permission::CanWrite,
                ObjectRef::new(ObjectType::Repository, fixture.private_repository),
            )
            .await
            .expect("owner write permission"),
        AuthorizationDecision::Deny
    );
    owner_tx.rollback().await.expect("rollback owner checks");

    let maintainer_identity = identity(fixture.maintainer);
    let mut maintainer_tx = begin_actor_transaction(&pool, &maintainer_identity)
        .await
        .expect("maintainer RLS transaction");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *maintainer_tx)
        .await
        .expect("use normal application role");
    let authorized_build: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM build_requests WHERE id = $1")
            .bind(fixture.build)
            .fetch_optional(&mut *maintainer_tx)
            .await
            .expect("authorized build request read");
    let authorized_execution: Option<Uuid> = sqlx::query_scalar(
        "SELECT build_request_id FROM build_executions WHERE build_request_id = $1",
    )
    .bind(fixture.build)
    .fetch_optional(&mut *maintainer_tx)
    .await
    .expect("authorized build execution read");
    let authorized_artifact: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM release_artifacts WHERE id = $1")
            .bind(fixture.artifact)
            .fetch_optional(&mut *maintainer_tx)
            .await
            .expect("authorized release artifact read");
    assert_eq!(authorized_build, Some(fixture.build));
    assert_eq!(authorized_execution, Some(fixture.build));
    assert_eq!(authorized_artifact, Some(fixture.artifact));
    maintainer_tx
        .rollback()
        .await
        .expect("rollback authorized RLS checks");

    let outsider_identity = identity(fixture.outsider);
    let mut outsider_tx = begin_actor_transaction(&pool, &outsider_identity)
        .await
        .expect("outsider actor transaction");
    assert_eq!(
        authorizer
            .check(
                &mut outsider_tx,
                Subject::User(fixture.outsider),
                Permission::CanRead,
                ObjectRef::new(ObjectType::Repository, fixture.private_repository),
            )
            .await
            .expect("private repository decision"),
        AuthorizationDecision::Deny
    );
    assert_eq!(
        authorizer
            .check(
                &mut outsider_tx,
                Subject::User(fixture.outsider),
                Permission::CanRead,
                ObjectRef::new(ObjectType::Repository, fixture.public_repository),
            )
            .await
            .expect("public repository decision"),
        AuthorizationDecision::Allow
    );
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *outsider_tx)
        .await
        .expect("use normal application role");
    let visible: Vec<String> = sqlx::query_scalar("SELECT name FROM repositories ORDER BY name")
        .fetch_all(&mut *outsider_tx)
        .await
        .expect("RLS repository list");
    assert_eq!(visible, vec![String::from("public")]);
    let private: Option<String> = sqlx::query_scalar("SELECT name FROM repositories WHERE id = $1")
        .bind(fixture.private_repository)
        .fetch_optional(&mut *outsider_tx)
        .await
        .expect("RLS direct read");
    assert!(private.is_none());
    let visible_releases: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM releases ORDER BY created_at, id")
            .fetch_all(&mut *outsider_tx)
            .await
            .expect("RLS release list");
    assert!(visible_releases.is_empty());
    let private_release: Option<Uuid> = sqlx::query_scalar("SELECT id FROM releases WHERE id = $1")
        .bind(fixture.release)
        .fetch_optional(&mut *outsider_tx)
        .await
        .expect("RLS direct release read");
    assert!(private_release.is_none());
    let private_build: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM build_requests WHERE id = $1")
            .bind(fixture.build)
            .fetch_optional(&mut *outsider_tx)
            .await
            .expect("RLS direct build request read");
    let private_execution: Option<Uuid> = sqlx::query_scalar(
        "SELECT build_request_id FROM build_executions WHERE build_request_id = $1",
    )
    .bind(fixture.build)
    .fetch_optional(&mut *outsider_tx)
    .await
    .expect("RLS direct build execution read");
    let private_artifact: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM release_artifacts WHERE id = $1")
            .bind(fixture.artifact)
            .fetch_optional(&mut *outsider_tx)
            .await
            .expect("RLS direct release artifact read");
    assert!(private_build.is_none());
    assert!(private_execution.is_none());
    assert!(private_artifact.is_none());
    let changed = sqlx::query("UPDATE repositories SET name = 'forbidden' WHERE id = $1")
        .bind(fixture.private_repository)
        .execute(&mut *outsider_tx)
        .await
        .expect("RLS silently filters update target");
    assert_eq!(changed.rows_affected(), 0);
    let deleted = sqlx::query("DELETE FROM repositories WHERE id = $1")
        .bind(fixture.private_repository)
        .execute(&mut *outsider_tx)
        .await
        .expect("RLS silently filters delete target");
    assert_eq!(deleted.rows_affected(), 0);
    let changed_release = sqlx::query("UPDATE releases SET version = 'forbidden' WHERE id = $1")
        .bind(fixture.release)
        .execute(&mut *outsider_tx)
        .await
        .expect("RLS silently filters release update target");
    assert_eq!(changed_release.rows_affected(), 0);
    let deleted_release = sqlx::query("DELETE FROM releases WHERE id = $1")
        .bind(fixture.release)
        .execute(&mut *outsider_tx)
        .await
        .expect("RLS silently filters release delete target");
    assert_eq!(deleted_release.rows_affected(), 0);
    sqlx::query("SAVEPOINT denied_repository_insert")
        .execute(&mut *outsider_tx)
        .await
        .expect("repository insert savepoint");
    let denied_insert = sqlx::query(
        "INSERT INTO repositories
         (id, project_id, name, default_branch, is_public)
         VALUES ($1, $2, 'forbidden', 'refs/heads/main', false)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.project)
    .execute(&mut *outsider_tx)
    .await;
    assert!(denied_insert.is_err());
    sqlx::query("ROLLBACK TO SAVEPOINT denied_repository_insert")
        .execute(&mut *outsider_tx)
        .await
        .expect("recover denied repository insert");
    sqlx::query("SAVEPOINT denied_release_insert")
        .execute(&mut *outsider_tx)
        .await
        .expect("release insert savepoint");
    let denied_release_insert = sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state)
         VALUES ($1, $2, 'forbidden', $3, 'refs/heads/main', $4, $5,
                 '{}', $6, $7, 'draft')",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.private_repository)
    .bind("b".repeat(40))
    .bind(fixture.build)
    .bind([11_u8; 32].as_slice())
    .bind([12_u8; 32].as_slice())
    .bind([13_u8; 32].as_slice())
    .execute(&mut *outsider_tx)
    .await;
    assert!(denied_release_insert.is_err());
    sqlx::query("ROLLBACK TO SAVEPOINT denied_release_insert")
        .execute(&mut *outsider_tx)
        .await
        .expect("recover denied release insert");
    outsider_tx.rollback().await.expect("rollback RLS checks");

    let member_identity = identity(fixture.member);
    let mut revocation_tx = begin_actor_transaction(&pool, &member_identity)
        .await
        .expect("member actor transaction");
    assert_eq!(
        authorizer
            .check(
                &mut revocation_tx,
                Subject::User(fixture.member),
                Permission::CanRead,
                ObjectRef::new(ObjectType::Repository, fixture.private_repository),
            )
            .await
            .expect("member read before revocation"),
        AuthorizationDecision::Allow
    );
    sqlx::query(
        "DELETE FROM organization_members
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(fixture.organization)
    .bind(fixture.member.as_uuid())
    .execute(&mut *revocation_tx)
    .await
    .expect("same-transaction membership revocation");
    assert_eq!(
        authorizer
            .check(
                &mut revocation_tx,
                Subject::User(fixture.member),
                Permission::CanRead,
                ObjectRef::new(ObjectType::Repository, fixture.private_repository),
            )
            .await
            .expect("member read after revocation"),
        AuthorizationDecision::Deny
    );
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member')",
    )
    .bind(fixture.organization)
    .bind(fixture.member.as_uuid())
    .execute(&mut *revocation_tx)
    .await
    .expect("same-transaction membership grant");
    assert_eq!(
        authorizer
            .check(
                &mut revocation_tx,
                Subject::User(fixture.member),
                Permission::CanRead,
                ObjectRef::new(ObjectType::Repository, fixture.private_repository),
            )
            .await
            .expect("member read after re-grant"),
        AuthorizationDecision::Allow
    );
    revocation_tx
        .rollback()
        .await
        .expect("rollback membership revocation");

    let maintainer_identity = identity(fixture.maintainer);
    let mut source_revocation_tx = begin_actor_transaction(&pool, &maintainer_identity)
        .await
        .expect("source revocation actor transaction");
    for (permission, object_type, object_id) in [
        (
            Permission::CanUse,
            ObjectType::ReleaseAgent,
            fixture.release_agent,
        ),
        (
            Permission::CanExecute,
            ObjectType::AgentAttachment,
            fixture.attachment,
        ),
        (
            Permission::CanUpdate,
            ObjectType::AgentInstance,
            fixture.instance,
        ),
    ] {
        assert_eq!(
            authorizer
                .check(
                    &mut source_revocation_tx,
                    Subject::User(fixture.maintainer),
                    permission,
                    ObjectRef::new(object_type, object_id),
                )
                .await
                .expect("permission before source revocation"),
            AuthorizationDecision::Allow
        );
    }
    sqlx::query(
        "DELETE FROM project_maintainers
         WHERE project_id = $1 AND user_id = $2",
    )
    .bind(fixture.project)
    .bind(fixture.maintainer.as_uuid())
    .execute(&mut *source_revocation_tx)
    .await
    .expect("revoke only source-project access");
    assert_eq!(
        authorizer
            .check(
                &mut source_revocation_tx,
                Subject::User(fixture.maintainer),
                Permission::CanUse,
                ObjectRef::new(ObjectType::ReleaseAgent, fixture.release_agent),
            )
            .await
            .expect("release use after source revocation"),
        AuthorizationDecision::Deny
    );
    for (permission, object_type, object_id) in [
        (
            Permission::CanExecute,
            ObjectType::AgentAttachment,
            fixture.attachment,
        ),
        (
            Permission::CanUpdate,
            ObjectType::AgentInstance,
            fixture.instance,
        ),
        (
            Permission::CanRead,
            ObjectType::AgentInstance,
            fixture.instance,
        ),
    ] {
        assert_eq!(
            authorizer
                .check(
                    &mut source_revocation_tx,
                    Subject::User(fixture.maintainer),
                    permission,
                    ObjectRef::new(object_type, object_id),
                )
                .await
                .expect("target or history permission after source revocation"),
            AuthorizationDecision::Allow
        );
    }
    source_revocation_tx
        .rollback()
        .await
        .expect("rollback source revocation");

    let unknown: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'unknown', 'repository', $2)")
            .bind(fixture.owner.to_string())
            .bind(fixture.private_repository.to_string())
            .fetch_one(&pool)
            .await
            .expect("unknown permission safely denied");
    assert_eq!(unknown, 0);
    let unknown_object: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_read', 'unknown', $2)")
            .bind(fixture.owner.to_string())
            .bind(fixture.private_repository.to_string())
            .fetch_one(&pool)
            .await
            .expect("unknown object safely denied");
    assert_eq!(unknown_object, 0);
    let app_bypass: bool =
        sqlx::query_scalar("SELECT rolbypassrls FROM pg_roles WHERE rolname = 'hephaestus_app'")
            .fetch_one(&pool)
            .await
            .expect("application role flags");
    assert!(!app_bypass);
    let app_owned_protected_tables: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM pg_class
         JOIN pg_roles ON pg_roles.oid = pg_class.relowner
         WHERE pg_class.relname IN
             ('projects', 'repositories', 'build_requests', 'releases',
              'release_artifacts', 'release_agents', 'agent_instances',
              'agent_instance_revisions', 'agent_attachments',
              'agent_updates', 'runs', 'agent_instance_state_volumes')
           AND pg_roles.rolname = 'hephaestus_app'",
    )
    .fetch_one(&pool)
    .await
    .expect("protected table ownership");
    assert_eq!(app_owned_protected_tables, 0);
}

async fn pool() -> Option<PgPool> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    Some(
        PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect to PostgreSQL"),
    )
}

fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://issuer.example",
        format!("subject-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}

#[allow(clippy::too_many_lines)]
fn parity_checks(fixture: &Fixture) -> Vec<ExpectedCheck> {
    let mut checks = Vec::with_capacity(77);
    let mut add = |subject, object_type, object_id, assertions: &[(Permission, bool)]| {
        checks.extend(
            assertions
                .iter()
                .copied()
                .map(|(permission, allowed)| ExpectedCheck {
                    subject,
                    permission,
                    object_type,
                    object_id,
                    allowed,
                }),
        );
    };

    add(
        fixture.owner,
        ObjectType::Organization,
        fixture.organization,
        &[
            (Permission::CanRead, true),
            (Permission::CanManageMembers, true),
            (Permission::CanCreateProject, true),
        ],
    );
    add(
        fixture.admin,
        ObjectType::Organization,
        fixture.organization,
        &[
            (Permission::CanRead, true),
            (Permission::CanManageMembers, true),
            (Permission::CanCreateProject, true),
            (Permission::CanDelete, false),
        ],
    );
    add(
        fixture.member,
        ObjectType::Organization,
        fixture.organization,
        &[
            (Permission::CanRead, true),
            (Permission::CanManageMembers, false),
            (Permission::CanCreateProject, false),
        ],
    );
    add(
        fixture.outsider,
        ObjectType::Organization,
        fixture.organization,
        &[(Permission::CanRead, false)],
    );
    add(
        fixture.member,
        ObjectType::Project,
        fixture.project,
        &[(Permission::CanRead, true), (Permission::CanWrite, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::Project,
        fixture.project,
        &[
            (Permission::CanRead, true),
            (Permission::CanWrite, true),
            (Permission::CanManage, true),
            (Permission::CanDelete, false),
        ],
    );
    add(
        fixture.maintainer,
        ObjectType::Repository,
        fixture.private_repository,
        &[
            (Permission::CanRead, true),
            (Permission::CanWrite, true),
            (Permission::CanDelete, false),
        ],
    );
    add(
        fixture.owner,
        ObjectType::Repository,
        fixture.private_repository,
        &[
            (Permission::CanRead, true),
            (Permission::CanWrite, false),
            (Permission::CanDelete, true),
        ],
    );
    add(
        fixture.outsider,
        ObjectType::Repository,
        fixture.private_repository,
        &[(Permission::CanRead, false)],
    );
    add(
        fixture.outsider,
        ObjectType::Repository,
        fixture.public_repository,
        &[(Permission::CanRead, true), (Permission::CanWrite, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::Build,
        fixture.build,
        &[
            (Permission::CanRead, true),
            (Permission::CanExecute, true),
            (Permission::CanCancel, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::Build,
        fixture.build,
        &[(Permission::CanRead, true), (Permission::CanExecute, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::Release,
        fixture.release,
        &[
            (Permission::CanRead, true),
            (Permission::CanPublish, true),
            (Permission::CanRevoke, true),
            (Permission::CanUse, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::Release,
        fixture.release,
        &[
            (Permission::CanRead, true),
            (Permission::CanPublish, false),
            (Permission::CanRevoke, false),
            (Permission::CanUse, true),
        ],
    );
    add(
        fixture.outsider,
        ObjectType::Release,
        fixture.release,
        &[
            (Permission::CanRead, false),
            (Permission::CanRevoke, false),
            (Permission::CanUse, false),
        ],
    );
    add(
        fixture.maintainer,
        ObjectType::ReleaseAgent,
        fixture.release_agent,
        &[(Permission::CanRead, true), (Permission::CanUse, true)],
    );
    add(
        fixture.outsider,
        ObjectType::ReleaseAgent,
        fixture.release_agent,
        &[(Permission::CanRead, false), (Permission::CanUse, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::AgentInstance,
        fixture.instance,
        &[
            (Permission::CanRead, true),
            (Permission::CanExecute, true),
            (Permission::CanManage, true),
            (Permission::CanUpdate, true),
            (Permission::CanRecover, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::AgentInstance,
        fixture.instance,
        &[
            (Permission::CanRead, true),
            (Permission::CanExecute, false),
            (Permission::CanUpdate, false),
            (Permission::CanRecover, false),
        ],
    );
    add(
        fixture.maintainer,
        ObjectType::AgentAttachment,
        fixture.attachment,
        &[
            (Permission::CanRead, true),
            (Permission::CanManage, true),
            (Permission::CanExecute, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::AgentAttachment,
        fixture.attachment,
        &[
            (Permission::CanRead, true),
            (Permission::CanManage, false),
            (Permission::CanExecute, false),
        ],
    );
    add(
        fixture.maintainer,
        ObjectType::AgentUpdate,
        fixture.update,
        &[(Permission::CanRead, true), (Permission::CanRecover, true)],
    );
    add(
        fixture.member,
        ObjectType::AgentUpdate,
        fixture.update,
        &[(Permission::CanRead, true), (Permission::CanRecover, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::Run,
        fixture.run,
        &[(Permission::CanRead, true), (Permission::CanCancel, true)],
    );
    add(
        fixture.member,
        ObjectType::Run,
        fixture.run,
        &[(Permission::CanRead, true), (Permission::CanCancel, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::StateVolume,
        fixture.volume,
        &[
            (Permission::CanRead, true),
            (Permission::CanAttach, true),
            (Permission::CanRestore, true),
            (Permission::CanManage, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::StateVolume,
        fixture.volume,
        &[
            (Permission::CanRead, true),
            (Permission::CanAttach, false),
            (Permission::CanRestore, false),
        ],
    );
    add(
        fixture.revoked,
        ObjectType::Project,
        fixture.project,
        &[(Permission::CanRead, false), (Permission::CanWrite, false)],
    );
    add(
        fixture.revoked,
        ObjectType::Repository,
        fixture.private_repository,
        &[(Permission::CanRead, false), (Permission::CanWrite, false)],
    );

    assert_eq!(checks.len(), 80, "fixture must match OpenFGA checks");
    checks
}

#[allow(clippy::too_many_lines)]
async fn seed(pool: &PgPool) -> Fixture {
    let fixture = Fixture {
        owner: UserId::new(),
        admin: UserId::new(),
        maintainer: UserId::new(),
        member: UserId::new(),
        outsider: UserId::new(),
        revoked: UserId::new(),
        organization: Uuid::new_v4(),
        project: Uuid::new_v4(),
        consuming_project: Uuid::new_v4(),
        private_repository: Uuid::new_v4(),
        public_repository: Uuid::new_v4(),
        consuming_repository: Uuid::new_v4(),
        build: Uuid::new_v4(),
        release: Uuid::new_v4(),
        artifact: Uuid::new_v4(),
        release_agent: Uuid::new_v4(),
        instance: Uuid::new_v4(),
        attachment: Uuid::new_v4(),
        update: Uuid::new_v4(),
        run: Uuid::new_v4(),
        volume: Uuid::new_v4(),
    };
    for (user, name) in [
        (fixture.owner, "owner"),
        (fixture.admin, "admin"),
        (fixture.maintainer, "maintainer"),
        (fixture.member, "member"),
        (fixture.outsider, "outsider"),
        (fixture.revoked, "revoked"),
    ] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(user.as_uuid())
            .bind(name)
            .execute(pool)
            .await
            .expect("seed user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'organization')")
        .bind(fixture.organization)
        .execute(pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner'), ($1, $3, 'admin'), ($1, $4, 'member')",
    )
    .bind(fixture.organization)
    .bind(fixture.owner.as_uuid())
    .bind(fixture.admin.as_uuid())
    .bind(fixture.member.as_uuid())
    .execute(pool)
    .await
    .expect("seed memberships");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
         VALUES ($1, $3, 'project'), ($2, $3, 'consuming-project')",
    )
    .bind(fixture.project)
    .bind(fixture.consuming_project)
    .bind(fixture.organization)
    .execute(pool)
    .await
    .expect("seed project");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $3), ($2, $3)",
    )
    .bind(fixture.project)
    .bind(fixture.consuming_project)
    .bind(fixture.maintainer.as_uuid())
    .execute(pool)
    .await
    .expect("seed maintainer");
    sqlx::query(
        "INSERT INTO repositories
         (id, project_id, name, default_branch, is_public)
         VALUES ($1, $3, 'private', 'refs/heads/main', false),
                ($2, $3, 'public', 'refs/heads/main', true),
                ($4, $5, 'consuming', 'refs/heads/main', false)",
    )
    .bind(fixture.private_repository)
    .bind(fixture.public_repository)
    .bind(fixture.project)
    .bind(fixture.consuming_repository)
    .bind(fixture.consuming_project)
    .execute(pool)
    .await
    .expect("seed repositories");
    let family = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let candidate_revision = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'agent')",
    )
    .bind(family)
    .bind(fixture.private_repository)
    .execute(pool)
    .await
    .expect("seed family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(fixture.build)
    .bind(fixture.private_repository)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed build");
    sqlx::query(
        "INSERT INTO build_executions
         (build_request_id, vm_id, release_id, release_agent_id,
          release_version, state, exit_code)
         VALUES ($1, $2, $3, $4, 'v1', 'drafted', 0)",
    )
    .bind(fixture.build)
    .bind(format!("vm-{}", fixture.build))
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .execute(pool)
    .await
    .expect("seed build execution");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5, '{}',
                 $6, $7, 'published', now())",
    )
    .bind(fixture.release)
    .bind(fixture.private_repository)
    .bind("a".repeat(40))
    .bind(fixture.build)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release");
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes,
          media_type, storage_key)
         VALUES ($1, $2, 'bin/agent', 'executable', 365, $3, 1,
                 'application/octet-stream', $4)",
    )
    .bind(fixture.artifact)
    .bind(fixture.release)
    .bind([5_u8; 32].as_slice())
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed release artifact");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'agent', 'Agent', $4, $5, true)",
    )
    .bind(fixture.release_agent)
    .bind(fixture.release)
    .bind(family)
    .bind(json!({
        "command": "bin/agent",
        "arguments": [],
        "working_directory": ".",
        "root_image_digest": "fixture"
    }))
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release agent");
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, 'agent', 'active')",
    )
    .bind(fixture.instance)
    .bind(fixture.consuming_project)
    .bind(family)
    .execute(pool)
    .await
    .expect("seed instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $7, $8, 'fixture/v1', true)",
    )
    .bind(revision)
    .bind(fixture.instance)
    .bind(fixture.release_agent)
    .bind([5_u8; 32].as_slice())
    .bind(json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind(json!({"network": "disabled"}))
    .bind(json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed revision");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $7, $8, 'fixture/v2', true)",
    )
    .bind(candidate_revision)
    .bind(fixture.instance)
    .bind(fixture.release_agent)
    .bind([7_u8; 32].as_slice())
    .bind(json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind(json!({"network": "disabled"}))
    .bind(json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind([8_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed candidate revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(fixture.instance)
        .bind(revision)
        .execute(pool)
        .await
        .expect("activate revision");
    sqlx::query(
        "INSERT INTO runs
         (id, instance_id, instance_revision_id, release_id, release_agent_id,
          run_kind, command_id, state, requires_state, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'update', $6, 'queued', true, now(), now())",
    )
    .bind(fixture.run)
    .bind(fixture.instance)
    .bind(revision)
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed run");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy, created_by)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'manual', $5)",
    )
    .bind(fixture.attachment)
    .bind(fixture.instance)
    .bind(fixture.consuming_project)
    .bind(fixture.consuming_repository)
    .bind(fixture.maintainer.as_uuid())
    .execute(pool)
    .await
    .expect("seed attachment");
    sqlx::query(
        "INSERT INTO agent_updates
         (id, instance_id, expected_current_revision_id,
          candidate_revision_id, state, actor_id)
         VALUES ($1, $2, $3, $4, 'candidate', $5)",
    )
    .bind(fixture.update)
    .bind(fixture.instance)
    .bind(revision)
    .bind(candidate_revision)
    .bind(fixture.maintainer.as_uuid())
    .execute(pool)
    .await
    .expect("seed update");
    sqlx::query(
        "INSERT INTO agent_instance_state_volumes
         (id, instance_id, host_id, host_path, capacity_bytes,
          filesystem_uuid, state)
         VALUES ($1, $2, 'host', $3, 16777216, $4, 'ready')",
    )
    .bind(fixture.volume)
    .bind(fixture.instance)
    .bind(format!("/tmp/{}.raw", fixture.volume))
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed state volume");
    fixture
}
