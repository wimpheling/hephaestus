//! Exactly three boundary registrations; the full22 denial/race matrix remains pending.
use super::{fixture, owning_transaction as owning};
use sqlx::{PgPool, Postgres, Transaction};

#[sqlx::test(migrations = false)]
#[ignore = "requires exact private28+selected29 PostgreSQL and genuine native Local births; controlled VM ports"]
async fn terminal_default_deferred_same_tx_cleanup_is_denied(pool: PgPool) {
    let f = Box::pin(fixture::running(&pool, 2)).await;
    let prepared = owning::prepare(&f).await;
    let mut tx = f.birth.birth.worker.begin().await.unwrap();
    owning::configure(&mut tx).await;
    let xid = owning::xid(&mut tx).await;
    owning::candidate(&mut tx, &f, &prepared.exit).await;
    assert_eq!(owning::xid(&mut tx).await, xid);
    // Default deferred timing remains untouched: the ordinary BEFORE guard must deny.
    owning::cleanup_denied(&mut tx, &f).await;
    tx.rollback().await.unwrap();
    prepared.unchanged(&f).await;
}

#[derive(Clone, Copy)]
enum ConstraintScope {
    All,
    Candidate,
}
async fn immediate(tx: &mut Transaction<'_, Postgres>, scope: ConstraintScope) {
    let query = match scope {
        ConstraintScope::All => sqlx::query("SET CONSTRAINTS ALL IMMEDIATE"),
        ConstraintScope::Candidate => {
            sqlx::query("SET CONSTRAINTS invocation119_terminal_candidate_commit IMMEDIATE")
        }
    };
    query.execute(&mut **tx).await.unwrap();
}
async fn deferred(tx: &mut Transaction<'_, Postgres>, scope: ConstraintScope) {
    let query = match scope {
        ConstraintScope::All => sqlx::query("SET CONSTRAINTS ALL DEFERRED"),
        ConstraintScope::Candidate => {
            sqlx::query("SET CONSTRAINTS invocation119_terminal_candidate_commit DEFERRED")
        }
    };
    query.execute(&mut **tx).await.unwrap();
}
#[sqlx::test(migrations = false)]
#[ignore = "requires exact private28+selected29 PostgreSQL and genuine native Local births; controlled VM ports"]
async fn terminal_forced_constraints_do_not_allow_same_tx_cleanup(pool: PgPool) {
    let f = Box::pin(fixture::running(&pool, 2)).await;
    let prepared = owning::prepare(&f).await;
    // Reuse only this same-run supplied wait result in this live worker process.
    for scope in [ConstraintScope::All, ConstraintScope::Candidate] {
        for (before_success, restore_deferred) in [(false, false), (true, false), (false, true)] {
            let mut tx = f.birth.birth.worker.begin().await.unwrap();
            owning::configure(&mut tx).await;
            let xid = owning::xid(&mut tx).await;
            if before_success {
                immediate(&mut tx, scope).await;
            }
            owning::candidate(&mut tx, &f, &prepared.exit).await;
            let candidate = owning::run(&mut tx, &f).await;
            let events = owning::events(&mut tx, &f).await;
            if !before_success {
                immediate(&mut tx, scope).await;
            }
            if restore_deferred {
                deferred(&mut tx, scope).await;
            }
            assert_eq!(owning::run(&mut tx, &f).await, candidate);
            assert_eq!(owning::events(&mut tx, &f).await, events);
            assert_eq!(owning::xid(&mut tx).await, xid);
            owning::cleanup_denied(&mut tx, &f).await;
            tx.rollback().await.unwrap();
            prepared.unchanged(&f).await;
        }
    }
}

#[derive(Clone, Copy)]
enum Child {
    Release,
    Nested,
    Rollback,
}
#[sqlx::test(migrations = false)]
#[ignore = "requires exact private28+selected29 PostgreSQL and genuine native Local births; controlled VM ports"]
async fn terminal_savepoint_release_is_not_success_commit(pool: PgPool) {
    let f = Box::pin(fixture::running(&pool, 2)).await;
    let prepared = owning::prepare(&f).await;
    for child in [Child::Release, Child::Nested, Child::Rollback] {
        let mut tx = f.birth.birth.worker.begin().await.unwrap();
        owning::configure(&mut tx).await;
        let xid = owning::xid(&mut tx).await;
        let original = owning::run(&mut tx, &f).await;
        let events = owning::events(&mut tx, &f).await;
        sqlx::query("SAVEPOINT terminal_outer")
            .execute(&mut *tx)
            .await
            .unwrap();
        if matches!(child, Child::Nested) {
            sqlx::query("SAVEPOINT terminal_inner")
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        owning::candidate(&mut tx, &f, &prepared.exit).await;
        assert_eq!(owning::xid(&mut tx).await, xid);
        match child {
            Child::Release => {}
            Child::Nested => {
                sqlx::query("RELEASE SAVEPOINT terminal_inner")
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                assert_eq!(owning::xid(&mut tx).await, xid);
            }
            Child::Rollback => {
                sqlx::query("ROLLBACK TO SAVEPOINT terminal_outer")
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                assert_eq!(owning::run(&mut tx, &f).await, original);
                assert_eq!(owning::events(&mut tx, &f).await, events);
                assert_eq!(owning::xid(&mut tx).await, xid);
                sqlx::query("RELEASE SAVEPOINT terminal_outer")
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                sqlx::query("SAVEPOINT terminal_retry")
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                // Ordinary marker birth must INSERT again; no ignored unique conflict or private read.
                owning::candidate(&mut tx, &f, &prepared.exit).await;
                assert_eq!(owning::xid(&mut tx).await, xid);
                sqlx::query("RELEASE SAVEPOINT terminal_retry")
                    .execute(&mut *tx)
                    .await
                    .unwrap();
            }
        }
        if !matches!(child, Child::Rollback) {
            sqlx::query("RELEASE SAVEPOINT terminal_outer")
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        assert_eq!(owning::xid(&mut tx).await, xid);
        let candidate = owning::run(&mut tx, &f).await;
        let success_events = owning::events(&mut tx, &f).await;
        immediate(&mut tx, ConstraintScope::All).await;
        assert_eq!(owning::run(&mut tx, &f).await, candidate);
        assert_eq!(owning::events(&mut tx, &f).await, success_events);
        assert_eq!(owning::xid(&mut tx).await, xid);
        owning::cleanup_denied(&mut tx, &f).await;
        tx.rollback().await.unwrap();
        prepared.unchanged(&f).await;
    }
}
