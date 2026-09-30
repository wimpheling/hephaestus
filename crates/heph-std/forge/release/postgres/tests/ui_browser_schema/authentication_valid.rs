use super::*;

// Keep the route matrix together so each accepted route remains visible next
// to its generation, gateway, and managed-service assertions.
#[allow(clippy::needless_borrow, clippy::too_many_lines)]
pub async fn assert_valid_routes(ctx: &AuthenticationContext) {
    let worker = ctx.worker.clone();
    let fixture = &ctx.fixture;
    let store = &ctx.store;
    let child_id = ctx.child_id;
    let valid_base = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await
        .expect("exact route base maps to entrypoint");
    assert_eq!(valid_base.session_id.as_uuid(), child_id);
    assert_eq!(valid_base.actor_id.as_uuid(), fixture.actor);
    assert_eq!(valid_base.organization_id.as_uuid(), fixture.organization);

    let valid_file = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui/index.html").expect("asset route"),
            },
        })
        .await
        .expect("published static asset authenticates");
    assert_eq!(valid_file.session_id.as_uuid(), child_id);

    let valid_static_api = authenticate_request(
        &store,
        fixture.generation,
        test_secret(fixture.actor, 90),
        UiBrowserRequestRoute::Api {
            route: RoutePath::parse("/service/api").expect("API route"),
            method: HttpMethod::Post,
        },
    )
    .await
    .expect("static UI API authenticates");
    assert_eq!(valid_static_api.session_id.as_uuid(), child_id);
    let global_child = insert_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.global_installation,
        fixture.global_generation,
        "schema-global",
        test_secret(fixture.actor, 95),
    )
    .await;
    let valid_global = authenticate_request(
        &store,
        fixture.global_generation,
        test_secret(fixture.actor, 95),
        UiBrowserRequestRoute::Static {
            route: UiBrowserRoute::parse("schema-global").expect("global route"),
        },
    )
    .await
    .expect("same-organization global UI authenticates");
    assert_eq!(valid_global.session_id.as_uuid(), global_child);
    assert_eq!(
        authenticate_request(
            &store,
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("API route"),
                method: HttpMethod::Get,
            },
        )
        .await,
        Err(UiBrowserSessionError::Unauthenticated)
    );

    let managed_child =
        insert_managed_authenticated_child(&worker, &fixture, test_secret(fixture.actor, 94)).await;
    let valid_managed = authenticate_request(
        &store,
        fixture.managed_generation,
        test_secret(fixture.actor, 94),
        UiBrowserRequestRoute::Managed {
            route: UiBrowserRoute::parse("schema-managed").expect("managed route"),
        },
    )
    .await
    .expect("managed UI authenticates");
    assert_eq!(valid_managed.session_id.as_uuid(), managed_child);
    let valid_managed_descendant = authenticate_request(
        &store,
        fixture.managed_generation,
        test_secret(fixture.actor, 94),
        UiBrowserRequestRoute::Managed {
            route: UiBrowserRoute::parse("schema-managed/child").expect("managed descendant"),
        },
    )
    .await
    .expect("managed UI descendant authenticates");
    assert_eq!(valid_managed_descendant.session_id.as_uuid(), managed_child);
    let valid_managed_api = authenticate_request(
        &store,
        fixture.managed_generation,
        test_secret(fixture.actor, 94),
        UiBrowserRequestRoute::Api {
            route: RoutePath::parse("/service/api").expect("managed API route"),
            method: HttpMethod::Post,
        },
    )
    .await
    .expect("managed UI API authenticates");
    assert_eq!(valid_managed_api.session_id.as_uuid(), managed_child);
    for request_route in [
        UiBrowserRequestRoute::Api {
            route: RoutePath::parse("/service/wrong").expect("wrong API route"),
            method: HttpMethod::Post,
        },
        UiBrowserRequestRoute::Api {
            route: RoutePath::parse("/service/api").expect("wrong method route"),
            method: HttpMethod::Get,
        },
    ] {
        assert_eq!(
            authenticate_request(
                &store,
                fixture.managed_generation,
                test_secret(fixture.actor, 94),
                request_route,
            )
            .await,
            Err(UiBrowserSessionError::Unauthenticated)
        );
    }
}
