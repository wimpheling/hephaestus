use super::*;

pub struct AuthenticationContext {
    pub worker: PgPool,
    pub app: PgPool,
    pub fixture: Fixture,
    pub store: PgUiBrowserSessionStore,
    pub child_id: Uuid,
    pub child_digest: Vec<u8>,
}

async fn setup_authentication() -> Option<AuthenticationContext> {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser authentication: test URL is unset");
        return None;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0097");
    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let child_digest = UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90))
        .digest()
        .as_bytes()
        .to_vec();
    let child_id =
        insert_authenticated_child(&worker, &fixture, child_digest.clone(), "1 hour").await;
    let store = PgUiBrowserSessionStore::new(worker.clone(), app.clone());

    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(fixture.outsider.to_string())
        .execute(&app)
        .await
        .expect("set spoofed application actor context");
    Some(AuthenticationContext {
        worker,
        app,
        fixture,
        store,
        child_id,
        child_digest,
    })
}

#[tokio::test]
#[serial]
async fn ui_browser_application_authentication_is_generation_and_route_bound() {
    let Some(ctx) = setup_authentication().await else {
        return;
    };
    assert_valid_routes(&ctx).await;
    assert_gateway_lifecycle(&ctx).await;
    assert_authentication_denials(&ctx).await;
    assert_authentication_authority(&ctx).await;
}
