//! Actual catalog capture only: no normalization, permitted deltas or reference
//! assertions. Sequence reads additionally require externally quiescent writers.
mod static_queries;

use serde::Deserialize;
use serde_json::Value;
use sqlx::{Acquire, PgConnection, PgPool};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Captured {
    json: Value,
    text: String,
}

impl Captured {
    pub const fn json(&self) -> &Value {
        &self.json
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    context: Captured,
    domains: BTreeMap<String, Captured>,
    sequences: BTreeMap<String, Captured>,
}

impl Snapshot {
    pub const fn context(&self) -> &Captured {
        &self.context
    }

    pub const fn domains(&self) -> &BTreeMap<String, Captured> {
        &self.domains
    }

    pub const fn sequences(&self) -> &BTreeMap<String, Captured> {
        &self.sequences
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Specification {
    format: String,
    #[serde(rename = "original_queries_SHA256")]
    original_queries_sha256: String,
    commands: Commands,
    context: String,
    queries: BTreeMap<String, String>,
    sequence_state: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Commands {
    isolation: String,
    timezone: String,
    search_path: String,
    rollback: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceName {
    oid: u32,
    schema: String,
    name: String,
    kind: String,
}

pub async fn capture(pool: &PgPool) -> Result<Snapshot, sqlx::Error> {
    let specification = specification()?;
    let mut connection = pool.acquire().await?;
    let mut tx = connection.begin().await?;
    let decision = async {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL TIME ZONE 'UTC'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL search_path TO pg_catalog, public, pg_temp")
            .execute(&mut *tx)
            .await?;
        let context = read(&mut tx, "__context", &specification.context).await?;
        check_context(context.json())?;
        let mut domains = BTreeMap::new();
        for (name, query) in &specification.queries {
            domains.insert(name.clone(), read(&mut tx, name, query).await?);
        }
        check_routine_kinds(&domains)?;
        let sequences = sequence_states(&mut tx, &specification, &domains).await?;
        Ok(Snapshot {
            context,
            domains,
            sequences,
        })
    }
    .await;
    match decision {
        Ok(snapshot) => {
            if let Err(error) = tx.commit().await {
                // This same retained connection gets an awaited cleanup attempt.
                // A disconnect can prevent it; return the original commit error.
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

async fn read(
    connection: &mut PgConnection,
    name: &str,
    expected: &str,
) -> Result<Captured, sqlx::Error> {
    let (json, text) = static_queries::read(connection, name, expected).await?;
    Ok(Captured { json, text })
}

fn check_context(context: &Value) -> Result<(), sqlx::Error> {
    for (key, expected) in [
        ("session_user", "postgres"),
        ("current_user", "postgres"),
        ("role", "none"),
        ("isolation", "repeatable read"),
        ("read_only", "on"),
        ("timezone", "UTC"),
        ("search_path", "pg_catalog, public, pg_temp"),
        ("server_version_num", "170010"),
    ] {
        require(
            context.get(key).and_then(Value::as_str) == Some(expected),
            "catalog requires qualified owner/17.10/RR/read-only/UTC/safe path context",
        )?;
    }
    for key in ["actor", "request", "subject_type", "occurrence"] {
        require(
            context.get(key).is_some_and(Value::is_null),
            "catalog requires unset event context",
        )?;
    }
    Ok(())
}

async fn sequence_states(
    connection: &mut PgConnection,
    specification: &Specification,
    domains: &BTreeMap<String, Captured>,
) -> Result<BTreeMap<String, Captured>, sqlx::Error> {
    let names: Vec<SequenceName> = serde_json::from_value(domains["sequence_names"].json.clone())
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    let metadata = domains["sequences"]
        .json
        .as_array()
        .ok_or_else(|| sqlx::Error::Protocol("sequence metadata is not an array".into()))?;
    let mut oids = BTreeSet::new();
    for row in metadata {
        let oid = sequence_oid(
            row.get("raw")
                .and_then(|raw| raw.get("seqrelid"))
                .ok_or_else(|| sqlx::Error::Protocol("sequence metadata lacks identity".into()))?,
        )?;
        require(oids.insert(oid), "duplicate sequence metadata identity")?;
    }
    require(
        oids.len() == names.len(),
        "sequence inventory differs from metadata",
    )?;
    let mut states = BTreeMap::new();
    for sequence in names {
        require(
            sequence.oid != 0
                && sequence.schema == "public"
                && sequence.kind == "S"
                && !sequence.name.is_empty()
                && oids.remove(&sequence.oid),
            "unverified sequence catalog identity",
        )?;
        let qualified = format!("{}.{}", quote(&sequence.schema), quote(&sequence.name));
        require(
            !states.contains_key(&qualified),
            "duplicate qualified sequence name",
        )?;
        // The sole interpolation is two quoted identifiers from the fixed
        // catalog/pg_sequence join. No caller or row supplies SQL fragments.
        // This fixed SQL template uses a literal token, not Rust formatting.
        #[allow(clippy::literal_string_with_formatting_args)]
        let query = specification
            .sequence_state
            .replace("{sequence}", &qualified);
        let (json, text): (Value, String) =
            sqlx::query_as(&query).fetch_one(&mut *connection).await?;
        states.insert(qualified, Captured { json, text });
    }
    require(oids.is_empty(), "uncaptured sequence metadata identity")?;
    Ok(states)
}

fn sequence_oid(value: &Value) -> Result<u32, sqlx::Error> {
    let text = value
        .as_str()
        .ok_or_else(|| sqlx::Error::Protocol("sequence OID is not a decimal string".into()))?;
    require(
        !text.is_empty()
            && !text.starts_with('0')
            && text.as_bytes().iter().all(u8::is_ascii_digit),
        "sequence OID is not canonical nonzero decimal",
    )?;
    text.parse()
        .map_err(|_| sqlx::Error::Protocol("sequence OID exceeds u32".into()))
}

fn check_routine_kinds(domains: &BTreeMap<String, Captured>) -> Result<(), sqlx::Error> {
    let routines = domains["routines"]
        .json
        .as_array()
        .ok_or_else(|| sqlx::Error::Protocol("routine capture is not an array".into()))?;
    for routine in routines {
        require(
            matches!(
                routine.get("kind").and_then(Value::as_str),
                Some("f" | "p" | "a" | "w")
            ),
            "unsupported routine kind",
        )?;
        if routine.get("kind").and_then(Value::as_str) == Some("a") {
            require(
                routine.get("aggregate").is_some_and(Value::is_object),
                "aggregate lacks raw aggregate metadata",
            )?;
        }
    }
    Ok(())
}

fn specification() -> Result<Specification, sqlx::Error> {
    let specification: Specification = serde_json::from_str(include_str!("catalog/queries.json"))
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    require(
        specification.format == "public_catalog_capture_v2"
            && specification.original_queries_sha256
                == "59a64bab3499736be09be26d7e36f4e3179c4cdb7265ef1e5175c5976f2105bb",
        "catalog query provenance changed",
    )?;
    require(
        specification.commands.isolation
            == "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY"
            && specification.commands.timezone == "SET LOCAL TIME ZONE 'UTC'"
            && specification.commands.search_path
                == "SET LOCAL search_path TO pg_catalog, public, pg_temp"
            && specification.commands.rollback == "ROLLBACK",
        "catalog commands changed",
    )?;
    require(
        specification.queries.keys().map(String::as_str).eq(DOMAINS),
        "catalog domain inventory changed",
    )?;
    require(
        specification.sequence_state
            == "SELECT payload,payload::text FROM (SELECT jsonb_build_object('last_value',last_value,'is_called',is_called) AS payload FROM {sequence}) AS capture",
        "sequence state template changed",
    )?;
    Ok(specification)
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

const DOMAINS: [&str; 39] = [
    "aggregates",
    "collations",
    "columns",
    "constraints",
    "database",
    "default_acl",
    "defaults",
    "dependencies",
    "descriptions",
    "enums",
    "extension_members",
    "extensions",
    "foreign_tables",
    "indexes",
    "inheritance",
    "initial_privileges",
    "languages",
    "memberships",
    "namespaces",
    "operator_classes",
    "operator_families",
    "operators",
    "partitioning",
    "policies",
    "ranges",
    "relations",
    "role_settings",
    "roles",
    "routines",
    "rules",
    "security_labels",
    "sequence_names",
    "sequences",
    "shared_dependencies",
    "shared_descriptions",
    "shared_security_labels",
    "triggers",
    "types",
    "views",
];
