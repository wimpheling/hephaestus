use super::*;

pub async fn assert_child_issue_interval_and_parent_cap(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(6),
    )
    .await
    .expect("insert valid handoff");
    assert_rejected_code(
        insert_child(worker, fixture, handoff, "60 seconds", "60 seconds"),
        "23000",
        "child issue at handoff expiry must fail",
    )
    .await;
    let before_child_handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(15),
    )
    .await
    .expect("insert handoff for child-issue ordering");
    let mut tx = worker
        .begin()
        .await
        .expect("begin child-issue ordering transaction");
    insert_child_tx(
        &mut tx,
        fixture,
        before_child_handoff,
        "30 seconds",
        "1 hour",
    )
    .await
    .expect("insert future-issued child");
    let before_child_error = sqlx::query(
        "UPDATE ui_browser_handoffs
         SET consumed_at = issued_at + interval '1 second' WHERE id = $1",
    )
    .bind(before_child_handoff)
    .execute(&mut *tx)
    .await
    .expect_err("consumption before child issue should fail");
    assert_database_code(
        &before_child_error,
        "23000",
        "consumption before child issue",
    );
    tx.rollback()
        .await
        .expect("rollback child-issue ordering rejection");
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(7),
    )
    .await
    .expect("insert second valid handoff");
    assert_rejected_code(
        insert_child(worker, fixture, handoff, "1 second", "13 hours"),
        "23514",
        "child expiry may not exceed the twelve-hour cap or parent expiry",
    )
    .await;
}

pub async fn assert_child_only_commit_rolls_back(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(8),
    )
    .await
    .expect("insert valid handoff");
    let mut tx = worker.begin().await.expect("begin worker transaction");
    insert_child_tx(&mut tx, fixture, handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before deferred rollback");
    let commit_error = tx
        .commit()
        .await
        .expect_err("deferred consumed trigger must reject child-only commit");
    assert_database_code(&commit_error, "23000", "child-only commit");
    let consumed: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(handoff)
            .fetch_one(worker)
            .await
            .expect("handoff remains after child-only rollback");
    assert!(consumed.is_none());
    let child_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(handoff)
            .fetch_one(worker)
            .await
            .expect("count child rows after rollback");
    assert_eq!(
        child_count, 0,
        "child-only rollback must leave no child row"
    );
}

pub async fn assert_consume_only_is_rejected(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(9),
    )
    .await
    .expect("insert valid handoff");
    assert_rejected_code(
        sqlx::query(
            "UPDATE ui_browser_handoffs
             SET consumed_at = statement_timestamp() WHERE id = $1",
        )
        .bind(handoff)
        .execute(worker),
        "23000",
        "handoff cannot be consumed without a child",
    )
    .await;
}

pub async fn assert_commit_and_consume_is_one_time(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(10),
    )
    .await
    .expect("insert valid handoff");
    let mut tx = worker.begin().await.expect("begin worker transaction");
    insert_child_tx(&mut tx, fixture, handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before one-time consume");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume matching child in same transaction");
    tx.commit()
        .await
        .expect("commit child and consume atomically");
    assert_rejected_code(
        insert_child(worker, fixture, handoff, "0 seconds", "1 hour"),
        "23000",
        "unique handoff_id prevents a second child",
    )
    .await;
    assert_rejected_code(
        sqlx::query(
            "UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1",
        )
        .bind(handoff)
        .execute(worker),
        "23000",
        "a consumed handoff cannot be reconsumed",
    )
    .await;
}

pub async fn assert_expiry_and_twelve_hour_cap(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        insert_handoff_with_times(worker, fixture, digest(11), "-61 seconds", "0 seconds"),
        "23514",
        "handoff lifetime must be exactly sixty seconds",
    )
    .await;
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(12),
    )
    .await
    .expect("insert valid handoff");
    let mut tx = worker.begin().await.expect("begin worker transaction");
    insert_child_tx(&mut tx, fixture, handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before expiry assertion");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume child before expiry assertion");
    tx.commit().await.expect("commit valid child and consume");
    let expired_handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(14),
    )
    .await
    .expect("insert second handoff for expiry timestamp");
    let mut tx = worker.begin().await.expect("begin expiry transaction");
    insert_child_tx(&mut tx, fixture, expired_handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before expiry rejection");
    let expiry_error = sqlx::query(
        "UPDATE ui_browser_handoffs
         SET consumed_at = issued_at + interval '60 seconds' WHERE id = $1",
    )
    .bind(expired_handoff)
    .execute(&mut *tx)
    .await
    .expect_err("consumption at expiry should fail before commit");
    assert_database_code(&expiry_error, "23000", "consumption at expiry");
    tx.rollback().await.expect("rollback expiry rejection");
}

pub async fn assert_application_role_is_denied(app: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        sqlx::query("SELECT * FROM ui_browser_handoffs").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("SELECT * FROM ui_browser_sessions").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("INSERT INTO ui_browser_handoffs DEFAULT VALUES").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("INSERT INTO ui_browser_sessions DEFAULT VALUES").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = now()").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("DELETE FROM ui_browser_handoffs").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("DELETE FROM ui_browser_sessions").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    let _ = fixture;
}

pub async fn assert_application_auth_tables_are_denied(app: &PgPool) {
    match sqlx::query("SELECT * FROM ui_browser_sessions")
        .fetch_all(app)
        .await
    {
        Ok(rows) => assert!(rows.is_empty(), "application role read session rows"),
        Err(error) => assert_database_code(&error, "42501", "application verifier table access"),
    }
    match sqlx::query("SELECT * FROM ui_browser_handoffs")
        .fetch_all(app)
        .await
    {
        Ok(rows) => assert!(rows.is_empty(), "application role read handoff rows"),
        Err(error) => assert_database_code(&error, "42501", "application verifier table access"),
    }
    match sqlx::query("SELECT * FROM human_browser_sessions")
        .fetch_all(app)
        .await
    {
        Ok(rows) => assert!(rows.is_empty(), "application role read parent session rows"),
        Err(error) => assert_database_code(&error, "42501", "application verifier table access"),
    }
    match sqlx::query("SELECT * FROM ui_installation_bindings")
        .fetch_all(app)
        .await
    {
        Ok(rows) => assert!(
            rows.is_empty(),
            "application role read installation bindings"
        ),
        Err(error) => assert_database_code(&error, "42501", "application verifier table access"),
    }
}

// The following helpers deliberately centralize the statement-time expressions
// so every matrix case remains comparable with the draft trigger checks.
