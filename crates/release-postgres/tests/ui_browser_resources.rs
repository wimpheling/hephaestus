//! Real-PostgreSQL checks for the generation-host and raw UI resource adapter.

#[path = "support/ui_browser_resource_fixture.rs"]
mod fixture;

use fixture::{
    insert_authenticated_child, insert_managed_authenticated_child,
    seed_fixture_reusing_installation_helpers, seed_fixture_reusing_installation_helpers_draft,
};
use gateway_domain::HttpMethod;
use identity_domain::RequestId;
use release_domain::{UiInstallationGenerationId, ui_browser::UiBrowserSessionSecret};
use release_postgres::{PgUiBrowserServingStore, PgUiGenerationHostResolver};
use release_service::{
    UiBrowserHttpRequest, UiBrowserHttpServingProjection, UiGatewayRequestKind, UiGenerationHost,
    UiGenerationHostResolver, UiNamespace, UiPublicPort, UiServingProjection,
};
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, time::Duration};
use uuid::Uuid;

const EXPECTED_MIGRATION: i64 = 92;

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_browser_resource_adapter_enforces_host_role_projection_and_current_auth() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser resources: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0092");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker");
    assert!(max_migration >= EXPECTED_MIGRATION);

    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
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

    for function_signature in [
        "public.resolve_active_ui_generation_host(uuid)",
        "public.resolve_ui_browser_resource(bytea,uuid,text,text)",
    ] {
        let (security_definer, executable, safe_search_path): (bool, bool, bool) = sqlx::query_as(
            "SELECT p.prosecdef,
                        has_function_privilege(current_user, p.oid, 'EXECUTE'),
                        EXISTS (
                            SELECT 1
                            FROM unnest(coalesce(p.proconfig, ARRAY[]::text[])) AS setting
                            WHERE setting = 'search_path=pg_catalog, pg_temp'
                        )
                 FROM pg_proc AS p
                 WHERE p.oid = to_regprocedure($1)",
        )
        .bind(function_signature)
        .fetch_one(&app)
        .await
        .expect("inspect application-role UI function contract");
        assert!(
            security_definer,
            "{function_signature} must be SECURITY DEFINER"
        );
        assert!(
            executable,
            "{function_signature} must grant EXECUTE to app role"
        );
        assert!(
            safe_search_path,
            "{function_signature} must pin search_path"
        );
    }

    let session_secret = UiBrowserSessionSecret::from_bytes([90; 32]);
    let child_id = insert_authenticated_child(
        &worker,
        &fixture,
        session_secret.digest().as_bytes().to_vec(),
        "1 hour",
    )
    .await;
    let store = PgUiBrowserServingStore::new(app.clone());
    let static_projection = store
        .authenticate_and_project_http(
            RequestId::new(),
            UiBrowserSessionSecret::from_bytes([90; 32]),
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
            UiBrowserSessionSecret::from_bytes([90; 32]),
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
            UiBrowserSessionSecret::from_bytes([90; 32]),
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
            UiBrowserSessionSecret::from_bytes([90; 32]),
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

    insert_managed_authenticated_child(&worker, &fixture, [94; 32]).await;
    let long_managed_path = format!("/schema-managed/{}", "x".repeat(260));
    let managed_projection = store
        .authenticate_and_project_http(
            RequestId::new(),
            UiBrowserSessionSecret::from_bytes([94; 32]),
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
            UiBrowserSessionSecret::from_bytes([94; 32]),
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
                UiBrowserSessionSecret::from_bytes([94; 32]),
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

#[tokio::test]
#[serial]
async fn ui_browser_resource_projection_fails_closed_on_static_api_ambiguity() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser ambiguity: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0092");
    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    let fixture = seed_fixture_reusing_installation_helpers_draft(&worker).await;
    sqlx::query(
        "INSERT INTO release_ui_api_bindings
         (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
         VALUES ($1, 'schema-ui', 'ambiguous', 'browser-service', 'GET', '/schema-ui', $2)",
    )
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .execute(&worker)
    .await
    .expect("seed colliding API declaration");
    sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key, release_id,
          ui_key, gateway_id, gateway_revision_id, release_agent_id, gateway_name,
          method, route, exposure)
         VALUES ($1, $2, 'api', 'ambiguous', $3, 'schema-ui', $4, $5, $6,
                 'browser-service', 'GET', '/schema-ui', 'heph_authenticated')",
    )
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.release)
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .bind(fixture.release_agent)
    .execute(&worker)
    .await
    .expect("seed colliding installation binding");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/schema-ui', ARRAY['GET'])",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.managed_revision)
    .bind(fixture.managed_gateway)
    .bind(fixture.source_project)
    .execute(&worker)
    .await
    .expect("seed colliding gateway route");
    sqlx::query(
        "UPDATE releases SET state = 'published', published_at = now(),
                publication_actor_id = $2 WHERE id = $1",
    )
    .bind(fixture.release)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("publish ambiguous release");
    let secret = UiBrowserSessionSecret::from_bytes([96; 32]);
    insert_authenticated_child(
        &worker,
        &fixture,
        secret.digest().as_bytes().to_vec(),
        "1 hour",
    )
    .await;
    let store = PgUiBrowserServingStore::new(app);
    let result = store
        .authenticate_and_project_http(
            RequestId::new(),
            secret,
            UiInstallationGenerationId::from_uuid(fixture.generation),
            UiBrowserHttpRequest::new(HttpMethod::Get, "/schema-ui").expect("ambiguous path"),
        )
        .await;
    assert!(result.is_err(), "ambiguous declarations must fail closed");
    println!("REAL_UI_BROWSER_RESOURCE_AMBIGUITY=1 exact_candidate_count=2 denied=1");
}

async fn role_pool(database_url: &str, role: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect({
            let role = role.to_owned();
            move |connection, _metadata| {
                let role = role.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('role', $1, false)")
                        .bind(role.clone())
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('application_name', $1, false)")
                        .bind(format!("ui-browser-resource-{role}"))
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
                        .bind(Uuid::nil().to_string())
                        .execute(&mut *connection)
                        .await
                        .map(|_| ())
                })
            }
        })
        .connect(database_url)
        .await
        .expect("connect restricted PostgreSQL role")
}
