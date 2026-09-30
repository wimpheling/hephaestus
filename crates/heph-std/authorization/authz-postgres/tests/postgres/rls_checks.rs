use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, begin_actor_transaction};
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{Fixture, identity};

// This phase intentionally keeps the authorized and outsider RLS reads/writes together.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn run(pool: &PgPool, fixture: &Fixture, authorizer: &PostgresMelangeAuthorizer) {
    let maintainer_identity = identity(fixture.maintainer);
    let mut maintainer_tx = begin_actor_transaction(pool, &maintainer_identity)
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
    let mut outsider_tx = begin_actor_transaction(pool, &outsider_identity)
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
    let visible: Vec<String> =
        sqlx::query_scalar("SELECT name FROM repositories WHERE project_id = $1 ORDER BY name")
            .bind(fixture.project)
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
    let visible_releases: Vec<Uuid> = sqlx::query_scalar(
        "SELECT releases.id
           FROM releases
           JOIN repositories ON repositories.id = releases.repository_id
          WHERE repositories.project_id = $1
          ORDER BY releases.created_at, releases.id",
    )
    .bind(fixture.project)
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
}
