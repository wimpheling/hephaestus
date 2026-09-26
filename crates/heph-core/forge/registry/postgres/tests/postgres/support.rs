use sqlx::{PgPool, postgres::PgPoolOptions};
use std::env;

pub async fn assert_platform_catalog_rls(pool: &PgPool, publication_id: uuid::Uuid) {
    let mut authenticated = pool.begin().await.expect("authenticated transaction");
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .execute(&mut *authenticated)
    .await
    .expect("set authenticated actor");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *authenticated)
        .await
        .expect("use application role");
    let visible =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM registry_publications WHERE id = $1")
            .bind(publication_id)
            .fetch_optional(&mut *authenticated)
            .await
            .expect("authenticated platform catalog read");
    assert_eq!(visible, Some(publication_id));
    authenticated
        .rollback()
        .await
        .expect("close authenticated transaction");

    let mut anonymous = pool.begin().await.expect("anonymous transaction");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *anonymous)
        .await
        .expect("use application role");
    let hidden =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM registry_publications WHERE id = $1")
            .bind(publication_id)
            .fetch_optional(&mut *anonymous)
            .await
            .expect("anonymous platform catalog read");
    assert!(hidden.is_none());
    anonymous
        .rollback()
        .await
        .expect("close anonymous transaction");
}

pub async fn seed_project_reader(
    pool: &PgPool,
    owner_id: uuid::Uuid,
    outsider_id: uuid::Uuid,
) -> uuid::Uuid {
    let organization_id = uuid::Uuid::new_v4();
    let project_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'registry-owner'), ($2, 'registry-outsider')")
        .bind(owner_id)
        .bind(outsider_id)
        .execute(pool)
        .await
        .expect("seed registry readers");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("registry-org-{}", organization_id.simple()))
        .execute(pool)
        .await
        .expect("seed registry organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("seed registry owner membership");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("registry-project-{}", project_id.simple()))
        .execute(pool)
        .await
        .expect("seed registry project");
    project_id
}

pub async fn assert_project_registry_rls(
    pool: &PgPool,
    owner_id: uuid::Uuid,
    outsider_id: uuid::Uuid,
    publication_id: uuid::Uuid,
) {
    assert_publication_visible_to(pool, owner_id, publication_id, true).await;
    assert_publication_visible_to(pool, outsider_id, publication_id, false).await;
}

async fn assert_publication_visible_to(
    pool: &PgPool,
    actor_id: uuid::Uuid,
    publication_id: uuid::Uuid,
    expected_visible: bool,
) {
    let mut transaction = pool.begin().await.expect("actor transaction");
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true)",
    )
    .bind(actor_id.to_string())
    .execute(&mut *transaction)
    .await
    .expect("set actor");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *transaction)
        .await
        .expect("use application role");
    let visible =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM registry_publications WHERE id = $1")
            .bind(publication_id)
            .fetch_optional(&mut *transaction)
            .await
            .expect("project publication read");
    assert_eq!(visible.is_some(), expected_visible);
    transaction
        .rollback()
        .await
        .expect("close actor transaction");
}

pub async fn fixture() -> Option<PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect registry test database");
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply registry migrations");
    assert!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT to_regclass('public.registry_publications')::text"
        )
        .fetch_one(&pool)
        .await
        .expect("inspect registry table")
        .is_some()
    );
    Some(pool)
}
