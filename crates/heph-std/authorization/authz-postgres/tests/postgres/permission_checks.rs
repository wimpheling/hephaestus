use authz_domain::{AuthorizationDecision, AuthzError, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, begin_actor_transaction};
use sqlx::PgPool;

use super::{
    parity::parity_checks,
    support::{Fixture, identity},
};

// This phase keeps the contextless, parity, and privileged checks in their original order.
#[allow(clippy::too_many_lines)]
pub async fn run(pool: &PgPool, fixture: &Fixture, authorizer: &PostgresMelangeAuthorizer) {
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

    for check in parity_checks(fixture) {
        let check_identity = identity(check.subject);
        let mut transaction = begin_actor_transaction(pool, &check_identity)
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
    let mut owner_tx = begin_actor_transaction(pool, &maintainer_identity)
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
    let mut owner_tx = begin_actor_transaction(pool, &owner_identity)
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
}
