use super::*;

pub async fn insert_handoff(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    digest: Vec<u8>,
) -> Result<Uuid, sqlx::Error> {
    insert_handoff_with_times_for(
        pool,
        fixture,
        organization,
        installation,
        generation,
        digest,
        "0 seconds",
        "60 seconds",
    )
    .await
}

pub async fn insert_handoff_with_times(
    pool: &PgPool,
    fixture: &Fixture,
    digest: Vec<u8>,
    issued_at: &str,
    expires_at: &str,
) -> Result<Uuid, sqlx::Error> {
    insert_handoff_with_times_for(
        pool,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest,
        issued_at,
        expires_at,
    )
    .await
}

// The matrix keeps each persisted binding dimension visible at the call site.
#[allow(clippy::too_many_arguments)]
pub async fn insert_handoff_with_times_for(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
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
    .bind(fixture.route)
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await
    .map(|_| id)
}

pub async fn insert_child(
    pool: &PgPool,
    fixture: &Fixture,
    handoff: Uuid,
    issued_at: &str,
    expires_at: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    insert_child_tx(&mut tx, fixture, handoff, issued_at, expires_at).await?;
    tx.commit().await
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

pub async fn insert_audit_child(pool: &PgPool, fixture: &Fixture) -> Uuid {
    let handoff_digest = UiBrowserHandoffSecret::random()
        .digest()
        .as_bytes()
        .to_vec();
    let handoff = insert_handoff(
        pool,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        handoff_digest,
    )
    .await
    .expect("insert audit child handoff");
    let child_id = Uuid::new_v4();
    let mut transaction = pool.begin().await.expect("begin audit child transaction");
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
        UiBrowserSessionSecret::random()
            .digest()
            .as_bytes()
            .to_vec(),
    )
    .bind(Uuid::new_v4())
    .bind(handoff)
    .execute(&mut *transaction)
    .await
    .expect("insert audit child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *transaction)
        .await
        .expect("consume audit child handoff");
    transaction.commit().await.expect("commit audit child");
    child_id
}
