//! Host, authentication, and projection checks for browser resources.

use super::{fixture, support};
use fixture::{
    fixture_session_secret, insert_authenticated_child, insert_authenticated_child_for,
    insert_managed_authenticated_child, seed_fixture_reusing_installation_helpers,
};
use gateway_domain::HttpMethod;
use identity_domain::RequestId;
use release_domain::{UiInstallationGenerationId, ui_browser::UiBrowserSessionSecret};
use release_postgres::{
    PgUiBrowserServingStore, PgUiGenerationHostResolver, UiBrowserTargetContextProjection,
};
use release_service::{
    UiBrowserHttpRequest, UiBrowserHttpServingProjection, UiGatewayRequestKind, UiGenerationHost,
    UiGenerationHostResolver, UiNamespace, UiPublicPort, UiServingProjection,
};
use serial_test::serial;
use std::env;
use uuid::Uuid;

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_browser_resource_adapter_enforces_host_role_projection_and_current_auth() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser resources: test URL is unset");
        return;
    };
    let (max_migration, worker, app) = support::connect_pools(&database_url).await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
    let port = UiPublicPort::https_default();
    let host = UiGenerationHost::from_generation_id(UiInstallationGenerationId::from_uuid(
        fixture.generation,
    ));
    let authority = host.authority(&namespace, port);
    assert_eq!(
        UiGenerationHost::parse(&authority, &namespace, port)
            .expect("canonical generation host")
            .generation_id(),
        host.generation_id()
    );
    let host_store = PgUiGenerationHostResolver::new(app.clone());
    assert_eq!(
        host_store
            .resolve_active_generation_host(host)
            .await
            .expect("host lookup")
            .expect("active host")
            .generation_id,
        host.generation_id()
    );
    let wrong_generation =
        UiGenerationHost::from_generation_id(UiInstallationGenerationId::from_uuid(Uuid::new_v4()));
    assert!(
        host_store
            .resolve_active_generation_host(wrong_generation)
            .await
            .expect("wrong host lookup")
            .is_none()
    );

    let direct_table_error = sqlx::query("SELECT id FROM public.ui_browser_sessions")
        .fetch_one(&app)
        .await
        .expect_err("application role must not read child rows");
    assert_eq!(
        direct_table_error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );

    support::assert_app_function_contracts(&app).await;

    let session_secret =
        UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 90));
    let child_id = insert_authenticated_child(
        &worker,
        &fixture,
        session_secret.digest().as_bytes().to_vec(),
        "1 hour",
    )
    .await;
    let store = PgUiBrowserServingStore::new(app.clone());
    let repository_secret =
        UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 96));
    let repository_child = insert_authenticated_child_for(
        &worker,
        &fixture,
        fixture.organization,
        fixture.repository_installation,
        fixture.repository_generation,
        "schema-repository",
        repository_secret.digest().as_bytes().to_vec(),
        "1 hour",
    )
    .await;
    let target = store
        .project_ui_target(
            RequestId::new(),
            repository_secret,
            UiInstallationGenerationId::from_uuid(fixture.repository_generation),
        )
        .await
        .expect("repository target context projection");
    assert_eq!(target.context.session_id.as_uuid(), repository_child);
    assert_eq!(target.repository_id.as_uuid(), fixture.repository);

    let global_secret =
        UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 97));
    insert_authenticated_child_for(
        &worker,
        &fixture,
        fixture.organization,
        fixture.global_installation,
        fixture.global_generation,
        "schema-global",
        global_secret.digest().as_bytes().to_vec(),
        "1 hour",
    )
    .await;
    assert!(
        store
            .project_ui_target(
                RequestId::new(),
                global_secret,
                UiInstallationGenerationId::from_uuid(fixture.global_generation),
            )
            .await
            .is_err(),
        "global UI must not project a repository target"
    );
    let static_projection = store
        .authenticate_and_project_http(
            RequestId::new(),
            UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 90)),
            UiInstallationGenerationId::from_uuid(fixture.generation),
            UiBrowserHttpRequest::new(HttpMethod::Get, "/schema-ui").expect("static path"),
        )
        .await
        .expect("static resource projection");
    match static_projection {
        UiServingProjection::Redirect { context, location } => {
            assert_eq!(context.session_id.as_uuid(), child_id);
            assert_eq!(location.as_str(), "/schema-ui/index.html");
        }
        UiServingProjection::Static { .. } => panic!("static base must redirect"),
        UiServingProjection::Gateway { .. } => panic!("static request projected as gateway"),
    }

    let static_file_projection = store
        .authenticate_and_project_http(
            RequestId::new(),
            UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 90)),
            UiInstallationGenerationId::from_uuid(fixture.generation),
            UiBrowserHttpRequest::new(HttpMethod::Get, "/schema-ui/index.html")
                .expect("static entrypoint path"),
        )
        .await
        .expect("static entrypoint projection");
    match static_file_projection {
        UiServingProjection::Static { artifact, .. } => {
            assert_eq!(
                artifact.cache_policy,
                release_domain::ui::UiCachePolicy::NoStore
            );
            assert_eq!(artifact.media_type.as_str(), "text/html");
        }
        UiServingProjection::Redirect { .. } => panic!("canonical static path redirected"),
        UiServingProjection::Gateway { .. } => panic!("static file projected as gateway"),
    }

    let static_head_projection = store
        .authenticate_and_project_http(
            RequestId::new(),
            UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 90)),
            UiInstallationGenerationId::from_uuid(fixture.generation),
            UiBrowserHttpRequest::new(HttpMethod::Head, "/schema-ui").expect("static HEAD path"),
        )
        .await
        .expect("static HEAD resource projection");
    match static_head_projection {
        UiServingProjection::Redirect { location, .. } => {
            assert_eq!(location.as_str(), "/schema-ui/index.html");
        }
        UiServingProjection::Static { .. } => panic!("static HEAD base must redirect"),
        UiServingProjection::Gateway { .. } => panic!("static HEAD projected as gateway"),
    }

    let api_projection = store
        .authenticate_and_project_http(
            RequestId::new(),
            UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 90)),
            UiInstallationGenerationId::from_uuid(fixture.generation),
            UiBrowserHttpRequest::new(HttpMethod::Post, "/service/api").expect("API path"),
        )
        .await
        .expect("API resource projection");
    match api_projection {
        UiServingProjection::Gateway { request, .. } => {
            assert_eq!(request.kind, UiGatewayRequestKind::Api);
            assert_eq!(request.path.as_str(), "/service/api");
            assert_eq!(request.method, HttpMethod::Post);
        }
        UiServingProjection::Redirect { .. } => panic!("API request must not redirect"),
        UiServingProjection::Static { .. } => panic!("API request projected as static"),
    }

    insert_managed_authenticated_child(
        &worker,
        &fixture,
        fixture_session_secret(fixture.actor, 94),
    )
    .await;
    let long_managed_path = format!("/schema-managed/{}", "x".repeat(260));
    let managed_projection = store
        .authenticate_and_project_http(
            RequestId::new(),
            UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 94)),
            UiInstallationGenerationId::from_uuid(fixture.managed_generation),
            UiBrowserHttpRequest::new(HttpMethod::Get, &long_managed_path)
                .expect("long managed path"),
        )
        .await
        .expect("managed descendant projection");
    match managed_projection {
        UiServingProjection::Gateway { request, .. } => {
            assert_eq!(request.kind, UiGatewayRequestKind::Managed);
            assert_eq!(request.path.as_str(), long_managed_path);
            assert_eq!(request.method, HttpMethod::Get);
        }
        UiServingProjection::Redirect { .. } => panic!("managed descendant must not redirect"),
        UiServingProjection::Static { .. } => panic!("managed request projected as static"),
    }

    let managed_base_projection = store
        .authenticate_and_project_http(
            RequestId::new(),
            UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 94)),
            UiInstallationGenerationId::from_uuid(fixture.managed_generation),
            UiBrowserHttpRequest::new(HttpMethod::Get, "/schema-managed")
                .expect("managed base path"),
        )
        .await
        .expect("managed base projection");
    match managed_base_projection {
        UiServingProjection::Redirect { location, .. } => {
            assert_eq!(location.as_str(), "/schema-managed/index.html");
        }
        UiServingProjection::Gateway { .. } => panic!("managed base must redirect"),
        UiServingProjection::Static { .. } => panic!("managed base projected as static"),
    }

    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.managed_gateway)
        .execute(&worker)
        .await
        .expect("pause managed gateway");
    assert!(
        store
            .authenticate_and_project_http(
                RequestId::new(),
                UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 94)),
                UiInstallationGenerationId::from_uuid(fixture.managed_generation),
                UiBrowserHttpRequest::new(HttpMethod::Get, "/schema-managed/index.html")
                    .expect("paused managed path"),
            )
            .await
            .is_err(),
        "paused gateway must not project managed UI"
    );
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.managed_gateway)
        .execute(&worker)
        .await
        .expect("restore managed gateway");

    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("suspend actor");
    let denied = store
        .authenticate_and_project_http(
            RequestId::new(),
            session_secret,
            UiInstallationGenerationId::from_uuid(fixture.generation),
            UiBrowserHttpRequest::new(HttpMethod::Get, "/schema-ui").expect("static path"),
        )
        .await;
    assert!(
        denied.is_err(),
        "suspended actor must lose serving authority"
    );
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore actor");

    sqlx::query("UPDATE ui_installations SET lifecycle = 'disabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("disable installation");
    assert!(
        host_store
            .resolve_active_generation_host(host)
            .await
            .expect("disabled host lookup")
            .is_none()
    );
    sqlx::query("UPDATE ui_installations SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("restore installation");

    assert_eq!(
        UiPublicPort::parse_text("0443"),
        Err(release_service::UiHostError::InvalidPort)
    );
    let _ = namespace;
    let _ = port;
    println!(
        "REAL_UI_BROWSER_RESOURCES=1 migration={max_migration} host=1 app_table_denial=1 static_redirect=1 static_cache=1 static_head_redirect=1 managed_base_redirect=1 managed_long_path=1 gateway_pause=1 account_revocation=1 lifecycle_host=1"
    );
}
