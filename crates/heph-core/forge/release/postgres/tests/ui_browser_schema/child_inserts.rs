use super::*;

pub async fn insert_authenticated_child_for_installation(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    session_secret: [u8; 32],
) -> Uuid {
    insert_authenticated_child_for_installation_with_expiry(
        pool,
        fixture,
        installation,
        generation,
        route,
        session_secret,
        false,
    )
    .await
}

pub async fn insert_expired_authenticated_child_for_installation(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    session_secret: [u8; 32],
) -> Uuid {
    insert_authenticated_child_for_installation_with_expiry(
        pool,
        fixture,
        installation,
        generation,
        route,
        session_secret,
        true,
    )
    .await
}

pub async fn insert_authenticated_child_for_installation_with_expiry(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
    route: &str,
    session_secret: [u8; 32],
    expired: bool,
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
    .expect("insert installation-bound authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin installation-bound child");
    let child_insert = if expired {
        sqlx::query(
            "INSERT INTO ui_browser_sessions
             (id, session_digest, request_id, handoff_id, parent_session_id,
              installation_id, generation_id, organization_id, route, issued_at, expires_at)
             SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                    handoff.installation_id, handoff.generation_id, handoff.organization_id,
                    handoff.route, handoff.issued_at,
                    handoff.issued_at + interval '1 second'
             FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
        )
    } else {
        sqlx::query(
            "INSERT INTO ui_browser_sessions
             (id, session_digest, request_id, handoff_id, parent_session_id,
              installation_id, generation_id, organization_id, route, issued_at, expires_at)
             SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                    handoff.installation_id, handoff.generation_id, handoff.organization_id,
                    handoff.route, handoff.issued_at, handoff.issued_at + interval '1 hour'
             FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
        )
    };
    child_insert
        .bind(child_id)
        .bind(
            UiBrowserSessionSecret::from_bytes(scoped_secret(fixture.actor, session_secret))
                .digest()
                .as_bytes()
                .to_vec(),
        )
        .bind(Uuid::new_v4())
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("insert installation-bound authentication child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume installation-bound authentication handoff");
    tx.commit()
        .await
        .expect("commit installation-bound authentication child");
    child_id
}

pub async fn insert_parent_with_times(
    pool: &PgPool,
    parent_id: Uuid,
    actor: Uuid,
    issued_at: &str,
    expires_at: &str,
) {
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6,
                 statement_timestamp() + $7::interval,
                 statement_timestamp() + $8::interval)",
    )
    .bind(parent_id)
    .bind(digest(202))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(203))
    .bind(actor)
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("insert parent with explicit timestamps");
}

pub async fn insert_authenticated_child_for_parent(
    pool: &PgPool,
    fixture: &Fixture,
    parent_session: Uuid,
    session_secret: [u8; 32],
    child_expiry: &str,
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
    .bind(digest(204))
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(parent_session)
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.organization)
    .bind(fixture.route)
    .execute(pool)
    .await
    .expect("insert parent-bound authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin parent-bound child");
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
    .bind(
        UiBrowserSessionSecret::from_bytes(scoped_secret(fixture.actor, session_secret))
            .digest()
            .as_bytes()
            .to_vec(),
    )
    .bind(Uuid::new_v4())
    .bind(handoff)
    .bind(child_expiry)
    .execute(&mut *tx)
    .await
    .expect("insert parent-bound authentication child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume parent-bound handoff");
    tx.commit().await.expect("commit parent-bound child");
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
        UiBrowserSessionSecret::from_bytes(scoped_secret(fixture.actor, session_secret))
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

// The negative cases intentionally override each durable binding independently.
#[allow(clippy::too_many_arguments)]
pub async fn insert_child_with_binding(
    pool: &PgPool,
    fixture: &Fixture,
    handoff: Uuid,
    parent_session: Uuid,
    installation: Uuid,
    generation: Uuid,
    organization: Uuid,
    session_digest: Vec<u8>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, $4, $5, $6, $7, $8,
                handoff.issued_at, handoff.issued_at + interval '1 hour'
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $9",
    )
    .bind(Uuid::new_v4())
    .bind(session_digest)
    .bind(Uuid::new_v4())
    .bind(parent_session)
    .bind(installation)
    .bind(generation)
    .bind(organization)
    .bind(fixture.route)
    .bind(handoff)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

pub async fn insert_child_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    _fixture: &Fixture,
    handoff: Uuid,
    issued_at: &str,
    expires_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                handoff.installation_id, handoff.generation_id, handoff.organization_id,
                handoff.route, handoff.issued_at + $5::interval,
                handoff.issued_at + $6::interval
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(Uuid::new_v4())
    .bind(digest(13))
    .bind(Uuid::new_v4())
    .bind(handoff)
    .bind(issued_at)
    .bind(expires_at)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}
