use super::*;

pub(crate) type SessionDiagnostic = (
    i32,
    String,
    String,
    Option<String>,
    Option<String>,
    Vec<i32>,
);

pub(crate) async fn wait_for_row_lock_waiters(
    pool: &PgPool,
    relation_name: &str,
    blocker_pid: i32,
    application_name: &str,
    minimum: i64,
    require_commit: bool,
) {
    for attempt in 0..200 {
        let (activity_waiters, owner_blockers, commit_waiters): (i64, i64, i64) = sqlx::query_as(
            "SELECT
                 (SELECT count(*) FROM pg_stat_activity
                  WHERE application_name = $1
                    AND pid <> pg_backend_pid()
                    AND wait_event_type = 'Lock'
                    AND cardinality(pg_blocking_pids(pid)) > 0),
                 (SELECT count(*) FROM pg_stat_activity
                  WHERE application_name = $1
                    AND pid <> pg_backend_pid()
                    AND wait_event_type = 'Lock'
                    AND $2 = ANY(pg_blocking_pids(pid))),
                 (SELECT count(*) FROM pg_stat_activity
                  WHERE application_name = $1
                    AND pid <> pg_backend_pid()
                    AND wait_event_type = 'Lock'
                    AND query ~* '^\\s*COMMIT')",
        )
        .bind(application_name)
        .bind(blocker_pid)
        .fetch_one(pool)
        .await
        .expect("read PostgreSQL row lock waiters");
        // PostgreSQL may queue the second worker behind the first worker, so
        // only one session can list the owner PID as its direct blocker. The
        // application name identifies only this test's spawned worker pool.
        if activity_waiters >= minimum
            && owner_blockers > 0
            && (!require_commit || commit_waiters >= minimum)
        {
            println!(
                "REAL_STATIC_INSTALL_LOCK_BARRIER=1 relation={relation_name} \
                 blocker_pid={blocker_pid} application_name={application_name} \
                 activity_waiters={activity_waiters} owner_blockers={owner_blockers} \
                 commit_waiters={commit_waiters} minimum={minimum}"
            );
            return;
        }
        if attempt == 199 {
            let named_sessions: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name = $1",
            )
            .bind(application_name)
            .fetch_one(pool)
            .await
            .expect("inspect named race sessions");
            println!(
                "REAL_STATIC_INSTALL_LOCK_TIMEOUT=1 relation={relation_name} \
                 blocker_pid={blocker_pid} application_name={application_name} \
                 named_sessions={named_sessions} activity_waiters={activity_waiters} \
                 owner_blockers={owner_blockers} commit_waiters={commit_waiters} \
                 minimum={minimum}"
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {minimum} row lock waiters on {relation_name}");
}

pub(crate) async fn wait_for_named_lock_waiters(
    pool: &PgPool,
    application_name: &str,
    minimum: i64,
) {
    for attempt in 0..200 {
        let (named_sessions, lock_waiters): (i64, i64) = sqlx::query_as(
            "SELECT
                 (SELECT count(*) FROM pg_stat_activity
                  WHERE application_name = $1 AND pid <> pg_backend_pid()),
                 (SELECT count(*) FROM pg_stat_activity
                  WHERE application_name = $1
                    AND pid <> pg_backend_pid()
                    AND wait_event_type = 'Lock'
                    AND cardinality(pg_blocking_pids(pid)) > 0)",
        )
        .bind(application_name)
        .fetch_one(pool)
        .await
        .expect("read PostgreSQL command-ledger waiters");
        if named_sessions >= minimum && lock_waiters >= minimum {
            println!(
                "REAL_UI_INSTALLATION_LEDGER_BARRIER=1 application_name={application_name} \
                 named_sessions={named_sessions} lock_waiters={lock_waiters} minimum={minimum}"
            );
            return;
        }
        if attempt == 199 {
            println!(
                "REAL_UI_INSTALLATION_LEDGER_TIMEOUT=1 application_name={application_name} \
                 named_sessions={named_sessions} lock_waiters={lock_waiters} minimum={minimum}"
            );
            let diagnostics: Vec<SessionDiagnostic> = sqlx::query_as(
                "SELECT pid, state, query, wait_event_type, wait_event, pg_blocking_pids(pid)
                     FROM pg_stat_activity
                     WHERE application_name = $1
                     ORDER BY pid",
            )
            .bind(application_name)
            .fetch_all(pool)
            .await
            .expect("inspect command-ledger sessions");
            for (pid, state, query, wait_type, wait_event, blockers) in diagnostics {
                println!(
                    "REAL_UI_INSTALLATION_LEDGER_SESSION=1 pid={pid} state={state:?} \
                     query={query:?} wait_type={wait_type:?} wait_event={wait_event:?} \
                     blockers={blockers:?}"
                );
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {minimum} named command-ledger lock waiters");
}
