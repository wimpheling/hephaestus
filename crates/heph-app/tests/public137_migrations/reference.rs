//! One SQLx-owned fresh143 reference for the populated test's lifetime.
//! C must preserve both matched names before launch and qualify native assets.
use super::{fixture, harness};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    PgPool, Postgres,
    testing::{TestArgs, TestSupport},
};
use std::collections::BTreeSet;

pub(super) async fn populated(target: &PgPool, test_path: &'static str) -> Result<(), sqlx::Error> {
    let args = TestArgs::new(test_path);
    let name = <Postgres as TestSupport>::db_name(&args);
    eprintln!("SQLx fresh143 reference registry name {name}; test path {test_path}");
    bind_source()?;
    // This is the pinned0.8.6 doc-hidden test API used by SQLx's own macro.
    // Its normal matched-name replacement requires C's prelaunch custody gate.
    let context = <Postgres as TestSupport>::test_context(&args).await?;
    assert_eq!(context.db_name, name);
    let reference = context.pool_opts.connect_with(context.connect_opts).await?;
    let result = harness::populated(target, &reference).await;
    reference.close().await;
    if result.is_ok() {
        <Postgres as TestSupport>::cleanup_test(&name).await?;
    }
    // Harness Err skips reference cleanup. Success cleanup errors propagate;
    // C must observe actual database/registry custody in that case. Panics do
    // not promise awaited close; establish inactivity before a failure archive.
    result
}

fn bind_source() -> Result<(), sqlx::Error> {
    let policy: Value = serde_json::from_str(include_str!("catalog_routines/policy.json"))
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    let sources = policy["source_sha256"]
        .as_object()
        .ok_or_else(|| sqlx::Error::Protocol("routine policy source map missing".into()))?;
    require(sources.len() == 36, "routine policy must pin108..143")?;
    for text in [
        include_str!("catalog_table_columns/policy.json"),
        include_str!("catalog_defaults/policy.json"),
        include_str!("catalog_constraints/policy.json"),
    ] {
        let policy: Value =
            serde_json::from_str(text).map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        require(
            policy["source_sha256"].as_object() == Some(sources),
            "catalog policy source map differs from qualified routine map",
        )?;
    }
    bind_constraints()?;
    let embedded = fixture::MIGRATIONS;
    require(
        embedded
            .iter()
            .map(|entry| entry.version)
            .collect::<Vec<_>>()
            == (1..=143).collect::<Vec<_>>(),
        "reference requires the exact full embedded143 inventory",
    )?;
    let mut versions = BTreeSet::new();
    for (filename, pin) in sources {
        let (prefix, suffix) = filename
            .strip_suffix(".sql")
            .and_then(|stem| stem.split_once('_'))
            .ok_or_else(|| sqlx::Error::Protocol("routine policy filename invalid".into()))?;
        let version: i64 = prefix
            .parse()
            .map_err(|error| sqlx::Error::Protocol(format!("routine policy version: {error}")))?;
        require(
            (108..=143).contains(&version)
                && prefix == format!("{version:04}")
                && versions.insert(version),
            "routine policy versions must be unique canonical108..143",
        )?;
        let migration = embedded
            .iter()
            .find(|entry| entry.version == version)
            .ok_or_else(|| sqlx::Error::Protocol("routine policy migration absent".into()))?;
        require(
            migration.description.as_ref() == suffix.replace('_', " "),
            "routine policy filename differs from embedded migration description",
        )?;
        let expected = pin
            .as_str()
            .ok_or_else(|| sqlx::Error::Protocol("routine policy digest not text".into()))?;
        let actual = format!("{:x}", Sha256::digest(migration.sql.as_bytes()));
        require(
            actual == expected,
            "routine policy SHA256 differs from complete embedded migration SQL",
        )?;
    }
    require(
        versions.into_iter().eq(108..=143),
        "routine policy108..143 inventory incomplete",
    )
}

fn bind_constraints() -> Result<(), sqlx::Error> {
    let policy: Value = serde_json::from_str(include_str!("catalog_constraints/policy.json"))
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    bind_constraint_baseline(&policy)?;
    let group = json!({"table": "runs", "kind": "c", "columns": ["attachment_id", "run_kind"],
        "foreign_table": null, "foreign_columns": []});
    require(
        policy["structural_removed"] == json!([group]),
        "constraint structural removal differs from Source120 witness",
    )?;
    for (inventory, name) in [
        ("baseline", Value::Null),
        ("additions", json!("run_kind_attachment_shape")),
    ] {
        let rules = policy[inventory].as_array().ok_or_else(|| {
            sqlx::Error::Protocol("constraint declaration inventory missing".into())
        })?;
        let selected: Vec<_> = rules.iter().filter(|rule| rule["group"] == group).collect();
        require(
            selected.len() == 1 && selected[0]["name"] == name,
            "constraint source witness needs one original group and its exact replacement",
        )?;
    }
    let migration = fixture::MIGRATIONS
        .iter()
        .find(|entry| entry.version == 120)
        .ok_or_else(|| sqlx::Error::Protocol("Source120 migration absent".into()))?;
    // The approved full witness occupies Source120 lines6..14. Full Source120
    // SQL is also bound below by the unchanged complete32-source gate.
    let statement = migration
        .sql
        .lines()
        .skip(5)
        .take(9)
        .collect::<Vec<_>>()
        .join("\n");
    let statement = statement
        .strip_suffix(';')
        .ok_or_else(|| sqlx::Error::Protocol("Source120 witness terminator missing".into()))?;
    require(
        format!("{:x}", Sha256::digest(statement.as_bytes()))
            == "02a07329ac3210e50f108ae1e7f8e58d37bb1862b6c7a929e383cd6ecf358197",
        "complete Source120 structural drop witness differs",
    )
}

fn bind_constraint_baseline(policy: &Value) -> Result<(), sqlx::Error> {
    let sources = policy["baseline_sha256"]
        .as_object()
        .ok_or_else(|| sqlx::Error::Protocol("constraint baseline source map missing".into()))?;
    require(sources.len() == 107, "constraint baseline must pin1..107")?;
    let mut versions = BTreeSet::new();
    for (filename, pin) in sources {
        let (prefix, suffix) = filename
            .strip_suffix(".sql")
            .and_then(|stem| stem.split_once('_'))
            .ok_or_else(|| sqlx::Error::Protocol("constraint baseline filename invalid".into()))?;
        let version: i64 = prefix.parse().map_err(|error| {
            sqlx::Error::Protocol(format!("constraint baseline version: {error}"))
        })?;
        require(
            (1..=107).contains(&version)
                && prefix == format!("{version:04}")
                && versions.insert(version),
            "constraint baseline versions must be unique canonical1..107",
        )?;
        let migration = fixture::MIGRATIONS
            .iter()
            .find(|entry| entry.version == version)
            .ok_or_else(|| sqlx::Error::Protocol("constraint baseline migration absent".into()))?;
        require(
            migration.description.as_ref() == suffix.replace('_', " "),
            "constraint baseline description differs from embedded migration",
        )?;
        let expected = pin
            .as_str()
            .ok_or_else(|| sqlx::Error::Protocol("constraint baseline digest not text".into()))?;
        require(
            format!("{:x}", Sha256::digest(migration.sql.as_bytes())) == expected,
            "constraint baseline SHA256 differs from complete embedded migration SQL",
        )?;
    }
    require(
        versions.into_iter().eq(1..=107),
        "constraint baseline inventory incomplete",
    )
}

fn require(value: bool, message: &str) -> Result<(), sqlx::Error> {
    if value {
        Ok(())
    } else {
        Err(sqlx::Error::Protocol(message.into()))
    }
}
