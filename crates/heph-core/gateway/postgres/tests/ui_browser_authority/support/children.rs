//! Browser handoff and child-session fixture rows.

use super::fixture::Fixture;
use sqlx::PgPool;
use uuid::Uuid;

pub fn digest(seed: u8) -> Vec<u8> {
    let mut value = vec![seed; 32];
    value[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    value
}

pub async fn insert_handoff(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    digest: Vec<u8>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 statement_timestamp(), statement_timestamp() + interval '60 seconds')",
    )
    .bind(id)
    .bind(digest)
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(installation)
    .bind(generation)
    .bind(organization)
    .bind(fixture.route)
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

pub async fn insert_authenticated_child_for_installation(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    session_digest: Vec<u8>,
) -> Uuid {
    let handoff = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route,
          issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 statement_timestamp(), statement_timestamp() + interval '60 seconds')",
    )
    .bind(handoff)
    .bind(digest(206))
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(installation)
    .bind(generation)
    .bind(fixture.organization)
    .bind(route)
    .execute(pool)
    .await
    .expect("insert unbound authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin unbound child");
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
    .bind(session_digest)
    .bind(Uuid::new_v4())
    .bind(handoff)
    .execute(&mut *tx)
    .await
    .expect("insert unbound authentication child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume unbound authentication handoff");
    tx.commit().await.expect("commit unbound child");
    child_id
}

pub async fn insert_managed_authenticated_child(
    pool: &PgPool,
    fixture: &Fixture,
    session_secret: [u8; 32],
) -> Uuid {
    insert_managed_authenticated_child_with_expiry(pool, fixture, session_secret, "1 hour").await
}

pub async fn insert_managed_authenticated_child_with_expiry(
    pool: &PgPool,
    fixture: &Fixture,
    session_secret: [u8; 32],
    child_expiry: &str,
) -> Uuid {
    let handoff = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
         installation_id, generation_id, organization_id, route,
         issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'docs',
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
                handoff.route, handoff.issued_at, handoff.issued_at + $5::interval
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(child_id)
    .bind(session_secret.to_vec())
    .bind(Uuid::new_v4())
    .bind(handoff)
    .bind(child_expiry)
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
