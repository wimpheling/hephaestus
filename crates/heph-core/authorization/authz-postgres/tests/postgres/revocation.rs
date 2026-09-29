use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, begin_actor_transaction};
use sqlx::PgPool;

use super::support::{Fixture, identity};

// This phase preserves the same-transaction revocation and source-project checks.
#[allow(clippy::too_many_lines)]
pub async fn run(pool: &PgPool, fixture: &Fixture, authorizer: &PostgresMelangeAuthorizer) {
    let member_identity = identity(fixture.member);
    let mut revocation_tx = begin_actor_transaction(pool, &member_identity)
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
    let mut source_revocation_tx = begin_actor_transaction(pool, &maintainer_identity)
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
}
