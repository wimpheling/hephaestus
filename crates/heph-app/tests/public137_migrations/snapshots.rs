//! Complete finite public-table multisets from one owner RR/read-only snapshot.
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Acquire, PgConnection, PgPool};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Public107,
    Public137,
    Public138,
    Public139,
    Public140,
    Public141,
    Public142,
    Public143,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Names {
    #[serde(rename = "source_contract_SHA256")]
    source_contract_sha256: String,
    public107_tables: Vec<String>,
    added_public137_tables: Vec<String>,
    views: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Column {
    pub name: String,
    pub position: i16,
    pub type_oid: i64,
    pub type_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub json: Value,
    // Exact PostgreSQL JSONB text also preserves numbers beyond JSON f64.
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Table {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    pub original_rows: Option<Vec<Row>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    phase: Phase,
    tables: BTreeMap<String, Table>,
    views: BTreeMap<String, Vec<Column>>,
}

impl Snapshot {
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    pub const fn tables(&self) -> &BTreeMap<String, Table> {
        &self.tables
    }

    pub const fn views(&self) -> &BTreeMap<String, Vec<Column>> {
        &self.views
    }
}

pub async fn capture(
    pool: &PgPool,
    phase: Phase,
    original: Option<&Snapshot>,
) -> Result<Snapshot, sqlx::Error> {
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
        collect(&mut tx, phase, original).await
    }
    .await;
    match decision {
        Ok(snapshot) => {
            if let Err(error) = tx.commit().await {
                // Retain this connection for an awaited cleanup attempt; a
                // disconnect can still prevent ROLLBACK from succeeding.
                let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
                return Err(error);
            }
            Ok(snapshot)
        }
        Err(error) => {
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

async fn context(connection: &mut PgConnection) -> Result<(), sqlx::Error> {
    let actual: Value = sqlx::query_scalar(
        "SELECT jsonb_build_array(session_user::text,current_user::text,
         current_setting('role'),current_setting('transaction_isolation'),
         current_setting('transaction_read_only'),current_setting('TimeZone'),
         NULLIF(current_setting('hephaestus.actor_id',true),''),
         NULLIF(current_setting('hephaestus.request_id',true),''),
         NULLIF(current_setting('hephaestus.subject_type',true),''),
         NULLIF(current_setting('hephaestus.occurrence_id',true),''))",
    )
    .fetch_one(connection)
    .await?;
    require(
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
        "snapshot requires actual owner, RR/read-only and unset event context",
    )
}

fn names(phase: Phase) -> Result<(BTreeSet<String>, BTreeSet<String>), sqlx::Error> {
    let names: Names = serde_json::from_str(include_str!("snapshots/table_names.json"))
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    require(
        names.source_contract_sha256
            == "e87e2d422d17d5ccfc6a2ff4adf0bd186fc6fbdac69eb04433a47193adb939a7",
        "snapshot name provenance mismatch",
    )?;
    let mut tables = names.public107_tables;
    if phase != Phase::Public107 {
        tables.extend(names.added_public137_tables);
    }
    if phase == Phase::Public143 {
        tables.push("recipe_prepared_instance_terminal_operations143".to_owned());
    }
    let table_count = tables.len();
    let view_count = names.views.len();
    let tables: BTreeSet<String> = tables.into_iter().collect();
    let views: BTreeSet<String> = names.views.into_iter().collect();
    require(
        tables.len() == table_count && views.len() == view_count,
        "duplicate declared table or view name",
    )?;
    let expected_count = match phase {
        Phase::Public107 => 153,
        Phase::Public143 => 184,
        _ => 183,
    };
    require(
        tables.len() == expected_count && views.len() == 10,
        "incomplete declared table or view sets",
    )?;
    Ok((tables, views))
}

async fn collect(
    connection: &mut PgConnection,
    phase: Phase,
    original: Option<&Snapshot>,
) -> Result<Snapshot, sqlx::Error> {
    if let Some(before) = original {
        require(
            before.phase == Phase::Public107,
            "projection needs Public107 baseline",
        )?;
    }
    let (tables, views) = names(phase)?;
    let relations: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.relname::text,c.relkind::text FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
         WHERE n.nspname='public' AND c.relkind IN ('r','p','v','m','f')
         ORDER BY c.relname COLLATE \"C\"",
    )
    .fetch_all(&mut *connection)
    .await?;
    let actual_tables: BTreeSet<String> = relations
        .iter()
        .filter(|(_, kind)| kind == "r" || kind == "p")
        .map(|(name, _)| name.clone())
        .collect();
    let actual_views: BTreeSet<String> = relations
        .iter()
        .filter(|(_, kind)| kind == "v")
        .map(|(name, _)| name.clone())
        .collect();
    require(tables == actual_tables, "public table name set mismatch")?;
    require(views == actual_views, "public view name set mismatch")?;
    require(
        relations.len() == tables.len() + views.len(),
        "unexpected public relation kind",
    )?;
    let mut snapshot = Snapshot {
        phase,
        tables: BTreeMap::new(),
        views: BTreeMap::new(),
    };
    for table in &tables {
        let columns = columns(connection, table).await?;
        if table == "_sqlx_migrations" {
            require(
                columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>()
                    == [
                        "version",
                        "description",
                        "installed_on",
                        "success",
                        "checksum",
                        "execution_time",
                    ],
                "SQLx journal must retain all six original columns",
            )?;
        }
        let rows = rows(connection, table, None).await?;
        let original_rows = if let Some(before) = original.and_then(|s| s.tables.get(table)) {
            require(
                before.columns.iter().all(|old| {
                    columns.iter().any(|c| {
                        c.name == old.name
                            && c.position == old.position
                            && c.type_oid == old.type_oid
                            && c.type_name == old.type_name
                    })
                }),
                "original column disappeared, moved or changed type",
            )?;
            Some(rows_projected(connection, table, &before.columns).await?)
        } else {
            None
        };
        snapshot.tables.insert(
            table.clone(),
            Table {
                columns,
                rows,
                original_rows,
            },
        );
    }
    for view in views {
        snapshot
            .views
            .insert(view.clone(), columns(connection, &view).await?);
    }
    Ok(snapshot)
}

async fn columns(connection: &mut PgConnection, table: &str) -> Result<Vec<Column>, sqlx::Error> {
    let found: Vec<(String, i16, i64, String)> = sqlx::query_as(
        "SELECT a.attname::text,a.attnum,a.atttypid::bigint,
         pg_catalog.format_type(a.atttypid,a.atttypmod)
         FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid
         JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
         WHERE n.nspname='public' AND c.relname=$1 AND a.attnum>0 AND NOT a.attisdropped
         ORDER BY a.attnum",
    )
    .bind(table)
    .fetch_all(connection)
    .await?;
    require(
        !found.is_empty(),
        "declared relation has no original columns",
    )?;
    Ok(found
        .into_iter()
        .map(|(name, position, type_oid, type_name)| Column {
            name,
            position,
            type_oid,
            type_name,
        })
        .collect())
}

async fn rows_projected(
    connection: &mut PgConnection,
    table: &str,
    columns: &[Column],
) -> Result<Vec<Row>, sqlx::Error> {
    // Columns come exclusively from a sealed, catalog-verified Public107
    // Snapshot. Callers cannot provide SQL, names or mutable baseline maps.
    let projection = columns
        .iter()
        .map(|column| quote(&column.name))
        .collect::<Vec<_>>()
        .join(",");
    rows(connection, table, Some(&projection)).await
}

async fn rows(
    connection: &mut PgConnection,
    table: &str,
    projection: Option<&str>,
) -> Result<Vec<Row>, sqlx::Error> {
    // Only finite names validated by collect() reach this private builder.
    // Quoting escapes identifiers; row values are never interpolated.
    let table = quote(table);
    let source = projection.map_or_else(
        || format!("public.{table}"),
        |columns| format!("(SELECT {columns} FROM public.{table})"),
    );
    let query = format!(
        "SELECT to_jsonb(snapshot_row),to_jsonb(snapshot_row)::text \
        FROM {source} AS snapshot_row \
        ORDER BY to_jsonb(snapshot_row)::text COLLATE \"C\""
    );
    let found: Vec<(Value, String)> = sqlx::query_as(&query).fetch_all(connection).await?;
    Ok(found
        .into_iter()
        .map(|(json, text)| Row { json, text })
        .collect())
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn require(condition: bool, message: &str) -> Result<(), sqlx::Error> {
    if condition {
        Ok(())
    } else {
        Err(sqlx::Error::Protocol(message.into()))
    }
}
