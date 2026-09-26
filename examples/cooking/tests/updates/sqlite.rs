// Reuse the update facade imports across focused lifecycle phases.
#[allow(unused_imports)]
use super::*;
/// Checks the committed `SQLite` migration in a host-visible state-volume
/// snapshot.  The path is supplied by the fixture because libkrun and local
/// backends expose different volume roots.  Python's `SQLite` client reads the
/// schema and rows through `SQLite` itself, including any associated WAL, so a
/// string search over raw database bytes cannot satisfy this assertion.
pub(crate) fn assert_migrated_sqlite(path: &Path, expected_recipe_count: usize) {
    let script = r#"
import sqlite3, sys
path, expected = sys.argv[1], int(sys.argv[2])
db = sqlite3.connect('file:' + path + '?mode=ro', uri=True)
version = db.execute('SELECT version FROM schema_meta').fetchone()[0]
assert version == 2, version
columns = {row[1] for row in db.execute('PRAGMA table_info(recipes)')}
assert 'recipe_summary' in columns, columns
indexes = {row[1] for row in db.execute('PRAGMA index_list(recipes)')}
assert 'recipes_summary' in indexes, indexes
recipes = db.execute('SELECT count(*) FROM recipes').fetchone()[0]
processed = db.execute('SELECT count(*) FROM processed_updates').fetchone()[0]
assert recipes == expected, (recipes, expected)
assert processed == expected, (processed, expected)
assert db.execute("SELECT count(*) FROM recipes WHERE context = ''").fetchone()[0] == 0
assert db.execute("SELECT count(*) FROM recipes WHERE recipe_summary = ''").fetchone()[0] == 0
if expected == 9:
    expected_ids = {f'recipe-{update_id}' for update_id in range(42, 51)}
    recipe_rows = db.execute(
        "SELECT recipe_id, publication_outcome, model_response IS NOT NULL, "
        "relay_outcome IS NOT NULL FROM recipes"
    ).fetchall()
    assert {row[0] for row in recipe_rows} == expected_ids
    for recipe_id, publication, has_model, has_relay in recipe_rows:
        if recipe_id == 'recipe-50':
            assert publication == 'pending'
            assert has_model
            assert not has_relay
        else:
            assert publication == 'proposal_ready'
            assert has_model
            assert has_relay
    dispositions = dict(db.execute(
        "SELECT update_id, disposition FROM processed_updates"
    ).fetchall())
    assert set(dispositions) == set(range(42, 51))
    assert all(dispositions[update_id] == 'completed' for update_id in range(42, 50))
    assert dispositions[50] == 'pending'
db.close()
"#;
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(path)
        .arg(expected_recipe_count.to_string())
        .output()
        .expect("run SQLite migration inspection");
    assert!(
        output.status.success(),
        "SQLite migration/schema conservation inspection failed"
    );
}

/// Extracts the committed state database from a detached ext4 volume and
/// validates it through `SQLite`. The volume must be detached before this is
/// called so `SQLite`'s WAL and main database are a consistent snapshot.
pub(crate) fn assert_migrated_sqlite_disk(disk: &Path, expected_recipe_count: usize) {
    assert!(
        disk.is_file(),
        "state-volume image is required for SQLite inspection"
    );
    let debugfs = std::process::Command::new("debugfs")
        .arg("-V")
        .output()
        .expect("debugfs is required for state-volume inspection");
    assert!(
        debugfs.status.success(),
        "debugfs prerequisite failed: {}",
        String::from_utf8_lossy(&debugfs.stderr)
    );
    let temporary = tempfile::tempdir().expect("create SQLite snapshot directory");
    let snapshot = temporary.path().join("cooking.sqlite3");
    let dump = |guest_path: &str, destination: &Path, required: bool| {
        let destination = destination.to_str().expect("SQLite snapshot path UTF-8");
        let output = std::process::Command::new("debugfs")
            .args(["-R", &format!("dump {guest_path} {destination}")])
            .arg(disk)
            .output()
            .expect("extract SQLite snapshot from state volume");
        if required {
            assert!(
                output.status.success(),
                "state volume SQLite extraction failed"
            );
            assert!(
                destination_path(destination),
                "SQLite snapshot is not a file"
            );
        }
    };
    dump("/cooking.sqlite3", &snapshot, true);
    // SQLite WAL mode may leave committed pages outside the main database.
    // Preserve both sidecars when present; the required main file check above
    // also prevents debugfs's zero exit status from hiding a missing path.
    dump(
        "/cooking.sqlite3-wal",
        &snapshot.with_extension("sqlite3-wal"),
        false,
    );
    dump(
        "/cooking.sqlite3-shm",
        &snapshot.with_extension("sqlite3-shm"),
        false,
    );
    super::super::cooking_confinement::assert_state_snapshot_has_no_credentials(temporary.path());
    assert_migrated_sqlite(&snapshot, expected_recipe_count);
}
