//! Exact `PostgreSQL` calendar arithmetic from journal108 and BEFORE intervals.
use super::{counters::Rule, rows};
use serde_json::{Value, json};
use sqlx::{Acquire, PgConnection, PgPool};
use std::collections::BTreeMap;
use time::OffsetDateTime;
use uuid::Uuid;

pub(super) struct Clocks {
    pub(super) clock: Value,
    pub(super) retained: BTreeMap<Uuid, Value>,
}

pub(super) async fn compute(
    pool: &PgPool,
    installed108: OffsetDateTime,
    rules: &BTreeMap<Uuid, Rule>,
) -> Result<Clocks, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let mut tx = connection.begin().await?;
    let decision = async {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL TIME ZONE 'UTC'")
            .execute(&mut *tx)
            .await?;
        context(&mut tx).await?;
        collect(&mut tx, installed108, rules).await
    }
    .await;
    match decision {
        Ok(clocks) => {
            if let Err(error) = tx.commit().await {
                let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
                return Err(error);
            }
            Ok(clocks)
        }
        Err(error) => {
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

async fn context(connection: &mut PgConnection) -> Result<(), sqlx::Error> {
    let actual: Value = sqlx::query_scalar(
        "SELECT pg_catalog.jsonb_build_array(session_user::text,current_user::text,
         pg_catalog.current_setting('role'),pg_catalog.current_setting('transaction_isolation'),
         pg_catalog.current_setting('transaction_read_only'),pg_catalog.current_setting('TimeZone'),
         NULLIF(pg_catalog.current_setting('hephaestus.actor_id',true),''),
         NULLIF(pg_catalog.current_setting('hephaestus.request_id',true),''),
         NULLIF(pg_catalog.current_setting('hephaestus.subject_type',true),''),
         NULLIF(pg_catalog.current_setting('hephaestus.occurrence_id',true),''))",
    )
    .fetch_one(connection)
    .await?;
    rows::require(
        actual
            == json!([
                "postgres",
                "postgres",
                "none",
                "repeatable read",
                "on",
                "UTC",
                null,
                null,
                null,
                null
            ]),
        "calendar oracle requires actual owner RR/read-only UTC with unset event context",
    )
}

async fn collect(
    connection: &mut PgConnection,
    installed108: OffsetDateTime,
    rules: &BTreeMap<Uuid, Rule>,
) -> Result<Clocks, sqlx::Error> {
    let clock: Value = sqlx::query_scalar("SELECT pg_catalog.to_jsonb($1::pg_catalog.timestamptz)")
        .bind(installed108)
        .fetch_one(&mut *connection)
        .await?;
    rows::require(
        clock.is_string(),
        "journal clock must have PostgreSQL UTC timestamp JSON",
    )?;
    let mut retained = BTreeMap::new();
    for (id, rule) in rules {
        // This reads no event/counter table: month/day/microsecond arithmetic
        // uses ONLY validated SQLx108 metadata plus the original BEFORE interval.
        let until: Value = sqlx::query_scalar(
            "SELECT pg_catalog.to_jsonb($1::pg_catalog.timestamptz OPERATOR(pg_catalog.+) ($2::pg_catalog.text)::pg_catalog.interval)",
        ).bind(installed108).bind(&rule.retention).fetch_one(&mut *connection).await?;
        rows::require(
            until.is_string(),
            "calendar retention must be timestamp JSON",
        )?;
        retained.insert(*id, until);
    }
    Ok(Clocks { clock, retained })
}
