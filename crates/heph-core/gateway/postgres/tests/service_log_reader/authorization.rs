//! Authorization and audit checks for the service log reader.

use super::support::{Fixture, identity};
use gateway_domain::{
    GatewayServiceLogReadCursor, GatewayServiceLogReadRequest, GatewayServiceLogReadScope,
};
use gateway_postgres::{GatewayServiceLogReaderError, PostgresGatewayServiceLogReader};
use identity_domain::AuthenticatedIdentity;
use uuid::Uuid;

// The phase keeps related authorization/snapshot assertions together.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn verify_authorization(
    admin: &sqlx::PgPool,
    reader: &PostgresGatewayServiceLogReader,
    fixture: &Fixture,
    scope: GatewayServiceLogReadScope,
    owner: &AuthenticatedIdentity,
    member: &AuthenticatedIdentity,
    outsider: &AuthenticatedIdentity,
) {
    let foreign_scope = GatewayServiceLogReadScope {
        project_id: fixture.other_project,
        ..scope
    };
    let foreign_cursor =
        GatewayServiceLogReadCursor::new(foreign_scope, 0).expect("foreign cursor scope");
    let invalid_cursor_request = GatewayServiceLogReadRequest {
        scope,
        limit: 100,
        after: Some(foreign_cursor),
    };
    assert!(matches!(
        reader.get_page(owner, invalid_cursor_request).await,
        Err(GatewayServiceLogReaderError::InvalidArgument)
    ));

    let wrong_project_page_identity = identity(fixture.owner, "wrong-project-page");
    assert_eq!(
        reader
            .get_page(
                &wrong_project_page_identity,
                GatewayServiceLogReadRequest::new(
                    GatewayServiceLogReadScope {
                        project_id: fixture.other_project,
                        ..scope
                    },
                    100,
                    None,
                )
                .expect("valid wrong-project page request"),
            )
            .await
            .map(|_| ()),
        Err(GatewayServiceLogReaderError::NotFound)
    );
    let wrong_project_page_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE request_id = $1 AND decision = 'allow'",
    )
    .bind(wrong_project_page_identity.request_id.as_uuid())
    .fetch_one(admin)
    .await
    .expect("read wrong-project page audits");
    assert_eq!(wrong_project_page_audits, 2);

    let future_page_identity = identity(fixture.owner, "future-page");
    assert_eq!(
        reader
            .get_page(
                &future_page_identity,
                GatewayServiceLogReadRequest::new(
                    GatewayServiceLogReadScope {
                        fencing_token: 3,
                        ..scope
                    },
                    100,
                    None,
                )
                .expect("valid future-fence page request"),
            )
            .await
            .map(|_| ()),
        Err(GatewayServiceLogReaderError::NotFound)
    );
    let future_page_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE request_id = $1 AND decision = 'allow'",
    )
    .bind(future_page_identity.request_id.as_uuid())
    .fetch_one(admin)
    .await
    .expect("read future-fence page audits");
    assert_eq!(future_page_audits, 2);

    let outsider_page_identity = identity(fixture.outsider, "outsider-page");
    assert_eq!(
        reader
            .get_page(
                &outsider_page_identity,
                GatewayServiceLogReadRequest::new(scope, 100, None)
                    .expect("valid outsider page request"),
            )
            .await
            .map(|_| ()),
        Err(GatewayServiceLogReaderError::Denied)
    );
    let outsider_page_denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE request_id = $1 AND decision = 'deny' AND object_type = 'project'",
    )
    .bind(outsider_page_identity.request_id.as_uuid())
    .fetch_one(admin)
    .await
    .expect("read outsider page audits");
    assert_eq!(outsider_page_denials, 1);

    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(fixture.project)
        .bind(fixture.member)
        .execute(admin)
        .await
        .expect("revoke member page access");
    let member_page_identity = identity(fixture.member, "member-page");
    assert_eq!(
        reader
            .get_page(
                &member_page_identity,
                GatewayServiceLogReadRequest::new(scope, 100, None)
                    .expect("valid revoked-member page request"),
            )
            .await
            .map(|_| ()),
        Err(GatewayServiceLogReaderError::Denied)
    );
    let member_page_denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE request_id = $1 AND decision = 'deny' AND object_type = 'project'",
    )
    .bind(member_page_identity.request_id.as_uuid())
    .fetch_one(admin)
    .await
    .expect("read revoked-member page audits");
    assert_eq!(member_page_denials, 1);

    sqlx::query(
        "DELETE FROM gateway_service_log_chunks
          WHERE instance_id = $1
            AND gateway_id = $2
            AND revision_id = $3
            AND project_id = $4
            AND fencing_token = 1",
    )
    .bind(fixture.instance)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.project)
    .execute(admin)
    .await
    .expect("remove fully evicted log payloads");
    sqlx::query(
        "UPDATE gateway_service_log_epochs
            SET retained_bytes = 0, retained_chunks = 0
          WHERE instance_id = $1
            AND gateway_id = $2
            AND revision_id = $3
            AND project_id = $4
            AND fencing_token = 1",
    )
    .bind(fixture.instance)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.project)
    .execute(admin)
    .await
    .expect("mark fully evicted log metadata");
    let fully_evicted = reader
        .get_page(
            owner,
            GatewayServiceLogReadRequest::new(scope, 100, None)
                .expect("valid fully evicted request"),
        )
        .await
        .expect("fully evicted page");
    assert!(fully_evicted.records.is_empty());
    assert!(fully_evicted.history_incomplete);
    assert_eq!(
        reader
            .get_epoch_metadata(member, scope)
            .await
            .expect_err("revoked member metadata must be denied"),
        GatewayServiceLogReaderError::Denied
    );

    let wrong_project_identity = identity(fixture.owner, "wrong-project");
    assert_eq!(
        reader
            .get_epoch_metadata(
                &wrong_project_identity,
                GatewayServiceLogReadScope {
                    project_id: fixture.other_project,
                    ..scope
                },
            )
            .await,
        Err(GatewayServiceLogReaderError::NotFound)
    );
    let wrong_project_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE request_id = $1 AND decision = 'allow'",
    )
    .bind(wrong_project_identity.request_id.as_uuid())
    .fetch_one(admin)
    .await
    .expect("read authorized not-found audit");
    assert_eq!(wrong_project_audits, 2);

    for mismatched in [
        GatewayServiceLogReadScope {
            project_id: Uuid::new_v4(),
            ..scope
        },
        GatewayServiceLogReadScope {
            gateway_id: Uuid::new_v4(),
            ..scope
        },
        GatewayServiceLogReadScope {
            revision_id: Uuid::new_v4(),
            ..scope
        },
        GatewayServiceLogReadScope {
            instance_id: Uuid::new_v4(),
            ..scope
        },
        GatewayServiceLogReadScope {
            fencing_token: 3,
            ..scope
        },
    ] {
        assert!(matches!(
            reader.get_epoch_metadata(owner, mismatched).await,
            Err(GatewayServiceLogReaderError::Denied | GatewayServiceLogReaderError::NotFound)
        ));
    }

    assert_eq!(
        reader.get_epoch_metadata(outsider, scope).await,
        Err(GatewayServiceLogReaderError::Denied)
    );
    assert_eq!(
        reader.get_epoch_metadata(member, scope).await,
        Err(GatewayServiceLogReaderError::Denied)
    );

    let (owner_allows, outsider_denies, member_denies): (i64, i64, i64) = sqlx::query_as(
        "SELECT
            count(*) FILTER (WHERE actor_id = $1 AND decision = 'allow'),
            count(*) FILTER (WHERE actor_id = $2 AND decision = 'deny'),
            count(*) FILTER (WHERE actor_id = $3 AND decision = 'deny')
         FROM authorization_audit_events
         WHERE actor_id IN ($1, $2, $3)",
    )
    .bind(fixture.owner)
    .bind(fixture.outsider)
    .bind(fixture.member)
    .fetch_one(admin)
    .await
    .expect("read persisted reader audit decisions");
    assert!(owner_allows >= 4);
    assert!(outsider_denies >= 1);
    assert!(member_denies >= 1);
}
