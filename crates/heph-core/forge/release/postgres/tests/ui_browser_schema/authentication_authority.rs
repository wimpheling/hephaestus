use super::*;

// Permission revocation and parent-session revocation form one authority
// phase; keeping the restore statements beside each assertion is intentional.
#[allow(clippy::needless_borrow, clippy::too_many_lines)]
pub async fn assert_authentication_authority(ctx: &AuthenticationContext) {
    let worker = ctx.worker.clone();
    let app = ctx.app.clone();
    let fixture = &ctx.fixture;
    let store = &ctx.store;
    for (generation, secret, request_route) in [
        (
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("source-revoked static route"),
            },
        ),
        (
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("source-revoked static API route"),
                method: HttpMethod::Post,
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Managed {
                route: UiBrowserRoute::parse("schema-managed")
                    .expect("source-revoked managed route"),
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("source-revoked managed API route"),
                method: HttpMethod::Post,
            },
        ),
    ] {
        assert_eq!(
            authenticate_request(&store, generation, secret, request_route).await,
            Err(UiBrowserSessionError::Unauthenticated)
        );
    }
    sqlx::query("INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')")
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("restore source organization membership");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(fixture.source_project)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore source project maintainer");
    println!(
        "REAL_UI_BROWSER_AUTH_SOURCE_TARGET=1 target_read=1 source_read=0 release_agent_use=1_to_0 source_denials=4 cross_project=1 global_positive=1"
    );

    let expired_parent = Uuid::new_v4();
    insert_parent_with_times(
        &worker,
        expired_parent,
        fixture.actor,
        "0 seconds",
        "+2 seconds",
    )
    .await;
    insert_authenticated_child_for_parent(
        &worker,
        &fixture,
        expired_parent,
        test_secret(fixture.actor, 92),
        "1 second",
    )
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 92)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    let future_parent = Uuid::new_v4();
    insert_parent_with_times(
        &worker,
        future_parent,
        fixture.actor,
        "+1 minute",
        "+2 hours",
    )
    .await;
    insert_authenticated_child_for_parent(
        &worker,
        &fixture,
        future_parent,
        test_secret(fixture.actor, 93),
        "1 hour",
    )
    .await;
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 93)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );

    sqlx::query(
        "UPDATE releases SET state = 'revoked', revoked_at = statement_timestamp() WHERE id = $1",
    )
    .bind(fixture.release)
    .execute(&worker)
    .await
    .expect("revoke release for verifier denial");
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 90)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    for (generation, secret, request_route) in [
        (
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("revoked static API route"),
                method: HttpMethod::Post,
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Managed {
                route: UiBrowserRoute::parse("schema-managed").expect("revoked managed route"),
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("revoked managed API route"),
                method: HttpMethod::Post,
            },
        ),
    ] {
        assert_eq!(
            authenticate_request(&store, generation, secret, request_route).await,
            Err(UiBrowserSessionError::Unauthenticated)
        );
    }
    sqlx::query(
        "UPDATE human_browser_sessions
     SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
     WHERE id = $1",
    )
    .bind(fixture.parent_session)
    .execute(&worker)
    .await
    .expect("revoke parent session");
    let revoked = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await;
    assert_eq!(revoked, Err(UiBrowserSessionError::Unauthenticated));

    assert_application_auth_tables_are_denied(&app).await;
}
