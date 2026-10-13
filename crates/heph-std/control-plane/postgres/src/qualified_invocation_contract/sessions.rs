use super::{Postgres, Transaction, invalid};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(FromRow)]
pub(super) struct SessionObservation {
    pid: i32,
    datid: i64,
    database: String,
    version: String,
    version_number: i32,
    login: String,
    current_role: String,
    selected_role: String,
    owner: String,
    superuser: bool,
    can_login: bool,
    bypass_rls: bool,
    isolation: String,
    read_only: String,
    search_path: String,
    timezone: String,
    event_context: Vec<Option<String>>,
    nonce: String,
}

pub(super) async fn observe(
    transaction: &mut Transaction<'_, Postgres>,
    expected_role: &str,
    selected_role: &str,
) -> Result<SessionObservation, sqlx::Error> {
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("SET LOCAL statement_timeout = '5s'")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("SET LOCAL lock_timeout = '1s'")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("SET LOCAL search_path TO pg_catalog, public, pg_temp")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("SET LOCAL TIME ZONE 'UTC'")
        .execute(&mut **transaction)
        .await?;
    // 62 ASCII bytes fit PostgreSQL's 63-byte application_name bound.
    let nonce = format!("heph-invocation-preflight:{}", Uuid::new_v4());
    let saved: String =
        sqlx::query_scalar("SELECT pg_catalog.set_config('application_name', $1, true)")
            .bind(&nonce)
            .fetch_one(&mut **transaction)
            .await?;
    if saved != nonce {
        return Err(invalid("transaction-local backend nonce differs"));
    }
    let context: SessionObservation = sqlx::query_as(
        "SELECT pg_catalog.pg_backend_pid() AS pid, database.oid::bigint AS datid,
                database.datname::text AS database,
                pg_catalog.current_setting('server_version') AS version,
                pg_catalog.current_setting('server_version_num')::integer AS version_number,
                session_user::text AS login, current_user::text AS current_role,
                pg_catalog.current_setting('role') AS selected_role,
                pg_catalog.pg_get_userbyid(database.datdba)::text AS owner,
                role.rolsuper AS superuser, role.rolcanlogin AS can_login,
                role.rolbypassrls AS bypass_rls,
                pg_catalog.current_setting('transaction_isolation') AS isolation,
                pg_catalog.current_setting('transaction_read_only') AS read_only,
                pg_catalog.current_setting('search_path') AS search_path,
                pg_catalog.current_setting('TimeZone') AS timezone,
                ARRAY[NULLIF(pg_catalog.current_setting('hephaestus.actor_id', true), ''),
                      NULLIF(pg_catalog.current_setting('hephaestus.request_id', true), ''),
                      NULLIF(pg_catalog.current_setting('hephaestus.subject_type', true), ''),
                      NULLIF(pg_catalog.current_setting('hephaestus.occurrence_id', true), '')]
                    AS event_context,
                pg_catalog.current_setting('application_name') AS nonce
         FROM pg_catalog.pg_database AS database
         JOIN pg_catalog.pg_roles AS role ON role.rolname = current_user
         WHERE database.datname = pg_catalog.current_database()",
    )
    .fetch_one(&mut **transaction)
    .await?;
    validate(&context, expected_role, selected_role, &nonce)?;
    Ok(context)
}

#[derive(FromRow)]
struct Backend {
    pid: i32,
    datid: i64,
    database: String,
    login: String,
    nonce: String,
    held: bool,
}

pub(super) async fn same_service(
    admin: &mut Transaction<'_, Postgres>,
    contexts: [&SessionObservation; 3],
) -> Result<(), sqlx::Error> {
    for context in contexts {
        if context.database != contexts[0].database
            || context.datid != contexts[0].datid
            || context.version != contexts[0].version
        {
            return Err(invalid(
                "held connections disagree on database or server version",
            ));
        }
    }
    let pids: Vec<i32> = contexts.iter().map(|context| context.pid).collect();
    if pids[0] == pids[1]
        || pids[0] == pids[2]
        || pids[1] == pids[2]
        || contexts[0].nonce == contexts[1].nonce
        || contexts[0].nonce == contexts[2].nonce
        || contexts[1].nonce == contexts[2].nonce
    {
        return Err(invalid("three distinct held backends required"));
    }
    let backends: Vec<Backend> = sqlx::query_as(
        "SELECT pid, datid::bigint AS datid, datname::text AS database,
                usename::text AS login, application_name::text AS nonce,
                (xact_start IS NOT NULL AND backend_type = 'client backend'
                 AND state IN ('active', 'idle in transaction')) AS held
         FROM pg_catalog.pg_stat_activity WHERE pid = ANY($1::integer[])",
    )
    .bind(&pids)
    .fetch_all(&mut **admin)
    .await?;
    if backends.len() != contexts.len()
        || contexts.iter().any(|context| {
            !backends.iter().any(|backend| {
                backend.pid == context.pid
                    && backend.datid == context.datid
                    && backend.database == context.database
                    && backend.login == context.login
                    && backend.nonce == context.nonce
                    && backend.held
            })
        })
    {
        return Err(invalid(
            "administrator cannot observe all exact held nonce/PID/database pairs",
        ));
    }
    Ok(())
}

fn validate(
    context: &SessionObservation,
    expected_role: &str,
    selected_role: &str,
    nonce: &str,
) -> Result<(), sqlx::Error> {
    if context.version_number / 10_000 != 17 {
        return Err(invalid(&format!(
            "held {expected_role} connection requires PostgreSQL major 17; observed server_version_num {}",
            context.version_number,
        )));
    }
    if context.pid <= 0
        || context.datid <= 0
        || context.login != "postgres"
        || context.current_role != expected_role
        || context.selected_role != selected_role
        || context.owner != "postgres"
        || context.superuser != (expected_role == "postgres")
        || context.can_login != (expected_role == "postgres")
        || context.bypass_rls != (expected_role != "hephaestus_app")
        || context.isolation != "repeatable read"
        || context.read_only != "on"
        || context.search_path != "pg_catalog, public, pg_temp"
        || context.timezone != "UTC"
        || context.event_context != [None, None, None, None]
        || context.nonce != nonce
    {
        return Err(invalid(&format!(
            "held {expected_role} connection role, owner, server version or context differs",
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "sessions/tests.rs"]
mod tests;
