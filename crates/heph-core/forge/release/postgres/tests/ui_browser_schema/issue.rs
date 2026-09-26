use super::*;

pub struct IssueContext {
    pub database_url: String,
    pub bootstrap: PgPool,
    pub worker: PgPool,
    pub store: PgUiBrowserSessionStore,
    pub fixture: Fixture,
    pub actor: UserId,
    pub parent: BrowserSessionId,
    pub installation: UiInstallationId,
    pub generation: UiInstallationGenerationId,
    pub route: UiBrowserRoute,
    pub baseline: i64,
    pub stale_generation: Uuid,
}

pub async fn run() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser issue: test URL is unset");
        return;
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
    let store = PgUiBrowserSessionStore::new(worker.clone(), app);
    let actor = UserId::from_uuid(fixture.actor);
    let parent = BrowserSessionId::from_uuid(fixture.parent_session);
    let installation = UiInstallationId::from_uuid(fixture.installation);
    let generation = UiInstallationGenerationId::from_uuid(fixture.generation);
    let route = UiBrowserRoute::parse("schema-ui").expect("published route base");
    let mut ctx = IssueContext {
        database_url,
        bootstrap,
        worker,
        store,
        fixture,
        actor,
        parent,
        installation,
        generation,
        route,
        baseline: 0,
        stale_generation: Uuid::nil(),
    };
    issue_success::run(&mut ctx).await;
    issue_barrier::run(&ctx).await;
    issue_authority::run(&mut ctx).await;
    issue_source::run(&ctx).await;
}
