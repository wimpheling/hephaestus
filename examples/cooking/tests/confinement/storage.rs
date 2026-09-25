use super::{
    CREDENTIAL_PATTERNS, assert_build_logs_have_no_credentials, assert_bytes_have_no_credentials,
    assert_vm_logs_have_no_credentials,
};
use futures_util::TryStreamExt as _;
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read as _,
    path::Path,
};

/// Scan the extracted `SQLite` database and any WAL/shared-memory sidecars.
/// Keep a suffix between reads so a credential spanning chunks is detected.
#[track_caller]
pub fn assert_state_snapshot_has_no_credentials(directory: &Path) {
    let longest = CREDENTIAL_PATTERNS
        .iter()
        .map(Vec::len)
        .max()
        .expect("fixture credentials");
    assert!(longest > 0);
    for name in [
        "cooking.sqlite3",
        "cooking.sqlite3-wal",
        "cooking.sqlite3-shm",
    ] {
        let path = directory.join(name);
        if name != "cooking.sqlite3" && !path.exists() {
            continue;
        }
        assert!(
            path.is_file(),
            "state scan requires a regular snapshot file"
        );
        let mut file = std::fs::File::open(path).expect("open extracted state snapshot");
        let mut chunk = [0_u8; 16_384];
        let mut window = Vec::with_capacity(chunk.len() + longest);
        loop {
            let count = file
                .read(&mut chunk)
                .expect("read extracted state snapshot");
            if count == 0 {
                break;
            }
            window.extend_from_slice(&chunk[..count]);
            assert_bytes_have_no_credentials(&window);
            let retained = window.len().saturating_sub(longest - 1);
            window.drain(..retained);
        }
    }
}

/// Verify static table coverage and execute the scan on a migrated fixture
/// database without requiring the full Cooking VM journey.
pub async fn assert_static_storage_scan_matches_catalog(pool: &PgPool) {
    let catalog: BTreeSet<String> = sqlx::query_scalar(
        "SELECT c.relname
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND c.relkind IN ('r', 'p')
           AND c.relname NOT IN ('_sqlx_migrations', 'melange_migrations')
         ORDER BY c.relname",
    )
    .fetch_all(pool)
    .await
    .expect("enumerate migrated storage surfaces")
    .into_iter()
    .collect();
    let covered: BTreeSet<_> = include_str!("../confinement.sql")
        .lines()
        .filter_map(|line| line.strip_prefix("SELECT '")?.split('\'').next())
        .map(str::to_owned)
        .collect();
    assert_eq!(
        covered, catalog,
        "update static storage scan for schema changes"
    );

    sqlx::query_as::<_, (String, String)>(include_str!("../confinement.sql"))
        .fetch_all(pool)
        .await
        .expect("execute static storage scan against migrated schema");
}

/// Scan stored rows through the fixture administrator pool supplied by the
/// golden harness. This administrative storage scan is independent of tenant
/// projections and the runtime worker role, so every public application table
/// remains visible even when product grants intentionally exclude it. It checks
/// row representations, including JSON payloads, outboxes, and whole-value
/// hexadecimal/base64 encodings of the fixture credentials; it does
/// not stand in for guest-memory, network, or browser-evidence checks.
pub async fn assert_database_has_no_credentials(pool: &PgPool) {
    let mut transaction = pool.begin().await.expect("begin raw storage scan");
    let (role, superuser, bypass_rls): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, r.rolsuper, r.rolbypassrls
         FROM pg_catalog.pg_roles AS r
         WHERE r.rolname = current_user",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("inspect fixture storage scan role");
    assert!(
        superuser || bypass_rls,
        "fixture storage scan requires rolsuper or rolbypassrls (current role: {role})"
    );
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND c.relkind IN ('r', 'p')
           AND c.relname NOT IN ('_sqlx_migrations', 'melange_migrations')
         ORDER BY c.relname",
    )
    .fetch_all(&mut *transaction)
    .await
    .expect("enumerate fixture storage surfaces");
    let unreadable: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND c.relkind IN ('r', 'p')
           AND c.relname NOT IN ('_sqlx_migrations', 'melange_migrations')
           AND NOT has_table_privilege(current_user, c.oid, 'SELECT')
         ORDER BY c.relname",
    )
    .fetch_all(&mut *transaction)
    .await
    .expect("check fixture storage surface visibility");
    assert!(
        unreadable.is_empty(),
        "fixture storage scan role cannot SELECT public tables: {}",
        unreadable.join(", ")
    );
    let patterns: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT pattern
         FROM unnest($1::text[]) AS fixture(value)
         CROSS JOIN LATERAL (VALUES
             (fixture.value),
             (encode(convert_to(fixture.value, 'UTF8'), 'hex')),
             (upper(encode(convert_to(fixture.value, 'UTF8'), 'hex'))),
             (replace(encode(convert_to(fixture.value, 'UTF8'), 'base64'), E'\\n', ''))
         ) AS encodings(pattern)",
    )
    .bind(super::super::cooking::FIXTURE_CREDENTIAL_SENTINELS.to_vec())
    .fetch_all(&mut *transaction)
    .await
    .expect("prepare transient credential scan patterns");
    // The query is static and reviewable. Compare its declared surfaces with
    // the live catalog so a new table cannot silently escape this scan.
    let covered: BTreeSet<_> = include_str!("../confinement.sql")
        .lines()
        .filter_map(|line| line.strip_prefix("SELECT '")?.split('\'').next())
        .collect();
    let catalog: BTreeSet<_> = tables.iter().map(String::as_str).collect();
    assert_eq!(
        covered, catalog,
        "update static storage scan for schema changes"
    );
    let mut counts = BTreeMap::<String, u64>::new();
    let mut rows = sqlx::query_as::<_, (String, String)>(include_str!("../confinement.sql"))
        .fetch(&mut *transaction);
    while let Some((surface, row)) = rows.try_next().await.expect("read raw fixture storage") {
        super::super::cooking::assert_no_credentials(&row);
        assert!(
            patterns.iter().all(|pattern| !row.contains(pattern)),
            "encoded fixture credential exposed in durable storage"
        );
        *counts.entry(surface).or_default() += 1;
    }
    drop(rows);
    for required in ["runs", "mailbox_events", "brokered_secret_lease_snapshots"] {
        assert!(
            counts.get(required).is_some_and(|count| *count > 0),
            "empty required scan surface: {required}"
        );
    }
    assert_vm_logs_have_no_credentials(&mut transaction).await;
    assert_build_logs_have_no_credentials(&mut transaction).await;
    let row_count: u64 = counts.values().sum();
    transaction
        .rollback()
        .await
        .expect("finish read-only storage scan");
    eprintln!(
        "Cooking raw database credential scan: {} tables, {row_count} rows",
        tables.len()
    );
}
