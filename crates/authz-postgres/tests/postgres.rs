//! Opt-in real-PostgreSQL tests for Mélange evaluation and RLS.

use authz_domain::{
    AuthorizationDecision, Authorizer, AuthzError, ObjectRef, ObjectType, Permission, Subject,
};
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
    private_repository: Uuid,
    public_repository: Uuid,
    agent: Uuid,
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
        (Permission::CanExecute, ObjectType::Agent, fixture.agent),
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
             ('projects', 'repositories', 'agents', 'runs', 'agent_state_volumes')
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
    let mut checks = Vec::with_capacity(46);
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
        ObjectType::Agent,
        fixture.agent,
        &[
            (Permission::CanRead, true),
            (Permission::CanExecute, true),
            (Permission::CanManage, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::Agent,
        fixture.agent,
        &[(Permission::CanRead, true), (Permission::CanExecute, false)],
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

    assert_eq!(checks.len(), 46, "fixture must match OpenFGA checks");
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
        private_repository: Uuid::new_v4(),
        public_repository: Uuid::new_v4(),
        agent: Uuid::new_v4(),
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
         VALUES ($1, $2, 'project')",
    )
    .bind(fixture.project)
    .bind(fixture.organization)
    .execute(pool)
    .await
    .expect("seed project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(fixture.project)
        .bind(fixture.maintainer.as_uuid())
        .execute(pool)
        .await
        .expect("seed maintainer");
    sqlx::query(
        "INSERT INTO repositories
         (id, project_id, name, default_branch, is_public)
         VALUES ($1, $3, 'private', 'refs/heads/main', false),
                ($2, $3, 'public', 'refs/heads/main', true)",
    )
    .bind(fixture.private_repository)
    .bind(fixture.public_repository)
    .bind(fixture.project)
    .execute(pool)
    .await
    .expect("seed repositories");
    sqlx::query("INSERT INTO agents (id, project_id, name) VALUES ($1, $2, 'agent')")
        .bind(fixture.agent)
        .bind(fixture.project)
        .execute(pool)
        .await
        .expect("seed agent");
    sqlx::query(
        "INSERT INTO runs
         (id, agent_id, command_id, state, created_at, updated_at)
         VALUES ($1, $2, $3, 'queued', now(), now())",
    )
    .bind(fixture.run)
    .bind(fixture.agent)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed run");
    sqlx::query(
        "INSERT INTO agent_state_volumes
         (id, agent_id, kind, host_id, host_path, capacity_bytes,
          filesystem_uuid, state)
         VALUES ($1, $2, 'agent_state', 'host', $3, 16777216, $4, 'ready')",
    )
    .bind(fixture.volume)
    .bind(fixture.agent)
    .bind(format!("/tmp/{}.raw", fixture.volume))
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed state volume");
    fixture
}
