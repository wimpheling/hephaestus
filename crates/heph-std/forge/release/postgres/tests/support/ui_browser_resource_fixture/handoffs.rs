use super::model::{Fixture, digest};
use release_domain::ui_browser::UiBrowserSessionSecret;
use sqlx::PgPool;
use uuid::Uuid;

async fn insert_handoff(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    digest: Vec<u8>,
) -> Result<Uuid, sqlx::Error> {
    insert_handoff_with_route(
        pool,
        fixture,
        organization,
        installation,
        generation,
        fixture.route,
        digest,
    )
    .await
}

async fn insert_handoff_with_route(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    digest: Vec<u8>,
) -> Result<Uuid, sqlx::Error> {
    insert_handoff_with_times_for(
        pool,
        fixture,
        organization,
        installation,
        generation,
        route,
        digest,
        "0 seconds",
        "60 seconds",
    )
    .await
}

// Handoff probes vary each persisted identity and both timestamps explicitly.
#[allow(clippy::too_many_arguments)]
async fn insert_handoff_with_times_for(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    digest: Vec<u8>,
    issued_at: &str,
    expires_at: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 statement_timestamp() + $10::interval,
                 statement_timestamp() + $11::interval)",
    )
    .bind(id)
    .bind(digest)
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(installation)
    .bind(generation)
    .bind(organization)
    .bind(route)
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await
    .map(|_| id)
}

pub async fn insert_authenticated_child(
    pool: &PgPool,
    fixture: &Fixture,
    session_digest: Vec<u8>,
    child_expiry: &str,
) -> Uuid {
    let handoff = insert_handoff(
        pool,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(90),
    )
    .await
    .expect("insert authentication handoff");
    insert_authenticated_child_for_handoff(pool, handoff, session_digest, child_expiry).await
}

// The helper keeps every installation identity explicit for scope-isolation tests.
#[allow(clippy::too_many_arguments)]
pub async fn insert_authenticated_child_for(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    session_digest: Vec<u8>,
    child_expiry: &str,
) -> Uuid {
    let handoff = insert_handoff_with_route(
        pool,
        fixture,
        organization,
        installation,
        generation,
        route,
        digest(90),
    )
    .await
    .expect("insert authentication handoff");
    insert_authenticated_child_for_handoff(pool, handoff, session_digest, child_expiry).await
}

async fn insert_authenticated_child_for_handoff(
    pool: &PgPool,
    handoff: Uuid,
    session_digest: Vec<u8>,
    child_expiry: &str,
) -> Uuid {
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin authentication child");
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                handoff.installation_id, handoff.generation_id, handoff.organization_id,
                handoff.route, handoff.issued_at, handoff.issued_at + $5::interval
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(child_id)
    .bind(session_digest)
    .bind(Uuid::new_v4())
    .bind(handoff)
    .bind(child_expiry)
    .execute(&mut *tx)
    .await
    .expect("insert authentication child");
    sqlx::query(
        "UPDATE ui_browser_handoffs
         SET consumed_at = statement_timestamp() WHERE id = $1",
    )
    .bind(handoff)
    .execute(&mut *tx)
    .await
    .expect("consume authentication handoff");
    tx.commit().await.expect("commit authentication child");
    child_id
}

pub async fn insert_managed_authenticated_child(
    pool: &PgPool,
    fixture: &Fixture,
    session_secret: [u8; 32],
) -> Uuid {
    let handoff = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route,
          issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'schema-managed',
                 statement_timestamp(), statement_timestamp() + interval '60 seconds')",
    )
    .bind(handoff)
    .bind(digest(205))
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(fixture.managed_installation)
    .bind(fixture.managed_generation)
    .bind(fixture.organization)
    .execute(pool)
    .await
    .expect("insert managed authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin managed child");
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                handoff.installation_id, handoff.generation_id, handoff.organization_id,
                handoff.route, handoff.issued_at, handoff.issued_at + interval '1 hour'
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(child_id)
    .bind(
        UiBrowserSessionSecret::from_bytes(session_secret)
            .digest()
            .as_bytes()
            .to_vec(),
    )
    .bind(Uuid::new_v4())
    .bind(handoff)
    .execute(&mut *tx)
    .await
    .expect("insert managed authentication child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume managed authentication handoff");
    tx.commit().await.expect("commit managed child");
    child_id
}
