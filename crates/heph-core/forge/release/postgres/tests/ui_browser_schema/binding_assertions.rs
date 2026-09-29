use super::*;

pub async fn authenticate_static(
    store: &PgUiBrowserSessionStore,
    fixture: &Fixture,
    secret: [u8; 32],
) -> Result<UiBrowserSessionContext, UiBrowserSessionError> {
    authenticate_request(
        store,
        fixture.generation,
        secret,
        UiBrowserRequestRoute::Static {
            route: UiBrowserRoute::parse("schema-ui").expect("route base"),
        },
    )
    .await
}

pub async fn authenticate_request(
    store: &PgUiBrowserSessionStore,
    generation: Uuid,
    secret: [u8; 32],
    request_route: UiBrowserRequestRoute,
) -> Result<UiBrowserSessionContext, UiBrowserSessionError> {
    store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(secret),
            expected_generation_id: UiInstallationGenerationId::from_uuid(generation),
            request_route,
        })
        .await
}

pub async fn assert_digest_constraints(worker: &PgPool, fixture: &Fixture) {
    let handoff = Uuid::new_v4();
    let bad_lengths = [vec![0_u8; 31], vec![0_u8; 33]];
    for digest in bad_lengths {
        assert_rejected_code(
            sqlx::query(
                "INSERT INTO ui_browser_handoffs
                 (id, handoff_digest, request_id, actor_id, parent_session_id,
                  installation_id, generation_id, organization_id, route,
                  issued_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                         statement_timestamp(), statement_timestamp() + interval '60 seconds')",
            )
            .bind(handoff)
            .bind(&digest)
            .bind(Uuid::new_v4())
            .bind(fixture.actor)
            .bind(fixture.parent_session)
            .bind(fixture.installation)
            .bind(fixture.generation)
            .bind(fixture.organization)
            .bind(fixture.route)
            .execute(worker),
            "23514",
            "handoff digest must be exactly 32 bytes",
        )
        .await;
    }
    let absent: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1")
            .bind(vec![0_u8; 32])
            .fetch_optional(worker)
            .await
            .expect("worker can perform a digest lookup for this matrix");
    assert!(
        absent.is_none(),
        "an unrelated digest must not authenticate"
    );
}

pub async fn assert_wrong_actor_binding(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        sqlx::query(
            "INSERT INTO ui_browser_handoffs
             (id, handoff_digest, request_id, actor_id, parent_session_id,
              installation_id, generation_id, organization_id, route,
              issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     statement_timestamp(), statement_timestamp() + interval '60 seconds')",
        )
        .bind(Uuid::new_v4())
        .bind(digest(1))
        .bind(Uuid::new_v4())
        .bind(fixture.outsider)
        .bind(fixture.parent_session)
        .bind(fixture.installation)
        .bind(fixture.generation)
        .bind(fixture.organization)
        .bind(fixture.route)
        .execute(worker),
        "23000",
        "actor must own the canonical parent session",
    )
    .await;
}

pub async fn assert_wrong_organization_binding(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        insert_handoff(
            worker,
            fixture,
            fixture.other_organization,
            fixture.installation,
            fixture.generation,
            digest(2),
        ),
        "23000",
        "organization must be derived from the installation target",
    )
    .await;
}

pub async fn assert_wrong_installation_generation_binding(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        insert_handoff(
            worker,
            fixture,
            fixture.organization,
            fixture.installation,
            fixture.other_generation,
            digest(3),
        ),
        "23503",
        "generation and installation must satisfy the composite FK",
    )
    .await;
    assert_rejected_code(
        insert_handoff(
            worker,
            fixture,
            fixture.organization,
            fixture.other_installation,
            fixture.generation,
            digest(4),
        ),
        "23503",
        "generation must belong to the selected installation",
    )
    .await;
}

pub async fn assert_wrong_child_binding(worker: &PgPool, fixture: &Fixture) {
    let cases = [
        (
            fixture.outsider_parent_session,
            fixture.installation,
            fixture.generation,
            fixture.organization,
            "parent actor/session binding",
        ),
        (
            fixture.parent_session,
            fixture.installation,
            fixture.generation,
            fixture.other_organization,
            "child organization binding",
        ),
        (
            fixture.parent_session,
            fixture.other_installation,
            fixture.other_generation,
            fixture.organization,
            "child installation/generation binding",
        ),
    ];
    for (parent_session, installation, generation, organization, reason) in cases {
        let handoff = insert_handoff(
            worker,
            fixture,
            fixture.organization,
            fixture.installation,
            fixture.generation,
            digest(20),
        )
        .await
        .expect("insert handoff for child binding case");
        assert_rejected_code(
            insert_child_with_binding(
                worker,
                fixture,
                handoff,
                parent_session,
                installation,
                generation,
                organization,
                digest(21),
            ),
            "23000",
            reason,
        )
        .await;
    }
}

pub async fn assert_initial_consumption_is_rejected(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        sqlx::query(
            "INSERT INTO ui_browser_handoffs
             (id, handoff_digest, request_id, actor_id, parent_session_id,
              installation_id, generation_id, organization_id, route,
              issued_at, expires_at, consumed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     statement_timestamp(), statement_timestamp() + interval '60 seconds',
                     statement_timestamp())",
        )
        .bind(Uuid::new_v4())
        .bind(digest(5))
        .bind(Uuid::new_v4())
        .bind(fixture.actor)
        .bind(fixture.parent_session)
        .bind(fixture.installation)
        .bind(fixture.generation)
        .bind(fixture.organization)
        .bind(fixture.route)
        .execute(worker),
        "23000",
        "consumed_at is a transition and cannot be supplied on INSERT",
    )
    .await;
}
