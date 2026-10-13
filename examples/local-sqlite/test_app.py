import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

import app


class SQLiteWorkloadTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.path = self.root / "app.sqlite"
        self.db = app.connect(self.path)
        self.addCleanup(self.db.close)
        app.initialize(self.db)

    def test_wal_commits_survive_reopen_with_another_connection(self):
        app.put(self.db, "key", "committed")
        self.assertTrue(Path(str(self.path) + "-wal").is_file())
        reopened = app.connect(self.path)
        try:
            self.assertEqual(reopened.execute("SELECT value FROM entries WHERE key=?", ("key",)).fetchone(), ("committed",))
            self.assertEqual(app.integrity(reopened), "ok")
            app.initialize(reopened)
            self.assertEqual(reopened.execute("SELECT count(*) FROM entries").fetchone()[0], 1)
        finally:
            reopened.close()

    def test_constraint_failure_rolls_back_overwrite_and_preserves_prior_data(self):
        app.put(self.db, "key", "before")
        self.db.execute("CREATE TRIGGER reject_value BEFORE UPDATE ON entries WHEN NEW.value='reject' BEGIN SELECT RAISE(ABORT,'rejected'); END")
        with self.assertRaises(sqlite3.IntegrityError):
            app.put(self.db, "key", "reject")
        self.assertFalse(self.db.in_transaction)
        self.assertEqual(self.db.execute("SELECT value FROM entries WHERE key='key'").fetchone()[0], "before")
        app.put(self.db, "key", "after")
        self.assertEqual(app.integrity(self.db), "ok")

    def test_process_crash_keeps_committed_wal_and_discards_open_transaction(self):
        code = """import os,sys,app
connection=app.connect(sys.argv[1])
app.initialize(connection)
app.put(connection,'crash','committed')
connection.execute('BEGIN IMMEDIATE')
connection.execute('UPDATE entries SET value=? WHERE key=?',('uncommitted','crash'))
os._exit(23)
"""
        result = subprocess.run([sys.executable, "-c", code, str(self.path)], cwd=Path(__file__).parent, timeout=10, check=False)
        self.assertEqual(result.returncode, 23)
        self.assertEqual(self.db.execute("SELECT value FROM entries WHERE key='crash'").fetchone()[0], "committed")
        self.assertEqual(app.integrity(self.db), "ok")

    def test_sql_strings_are_literal_values_not_executable_statements(self):
        text = "'); DROP TABLE entries; --"
        app.put(self.db, text, text)
        self.assertEqual(self.db.execute("SELECT value FROM entries WHERE key=?", (text,)).fetchone(), (text,))
        self.assertEqual(app.integrity(self.db), "ok")

    def test_backup_contains_committed_wal_not_concurrent_uncommitted_change(self):
        app.put(self.db, "key", "committed")
        writer = app.connect(self.path)
        self.addCleanup(writer.close)
        writer.execute("BEGIN IMMEDIATE")
        writer.execute("UPDATE entries SET value='not committed'")
        backup = self.root / "backup.sqlite"
        prepared = app.prepare_backup(self.db, backup)
        writer.rollback()
        self.assertEqual(prepared["rows"], 1)
        self.assertEqual(len(prepared["sha256"]), 64)
        # Restore/query a separate database through SQLite's own backup API.
        snapshot = sqlite3.connect(backup)
        restored = sqlite3.connect(self.root / "restored.sqlite")
        try:
            snapshot.backup(restored)
            self.assertEqual(restored.execute("SELECT value FROM entries WHERE key='key'").fetchone(), ("committed",))
            self.assertEqual(app.integrity(restored), "ok")
            app.put(self.db, "key", "newer source")
            self.assertEqual(restored.execute("SELECT value FROM entries WHERE key='key'").fetchone(), ("committed",))
        finally:
            snapshot.close()
            restored.close()

    def test_existing_backup_is_never_overwritten(self):
        backup = self.root / "backup.sqlite"
        app.prepare_backup(self.db, backup)
        before = backup.read_bytes()
        app.put(self.db, "new", "data")
        with self.assertRaises(FileExistsError):
            app.prepare_backup(self.db, backup)
        self.assertEqual(backup.read_bytes(), before)

    def test_full_dataset_rejects_growth_but_allows_existing_key_update(self):
        self.db.execute("BEGIN IMMEDIATE")
        self.db.executemany("INSERT INTO entries(key,value) VALUES(?,?)", [(str(index), "old") for index in range(app.MAX_ROWS)])
        self.db.commit()
        with self.assertRaisesRegex(ValueError, "entry_limit"):
            app.put(self.db, "extra", "denied")
        app.put(self.db, "0", "updated")
        self.assertEqual(self.db.execute("SELECT count(*) FROM entries").fetchone()[0], app.MAX_ROWS)

    def test_newer_schema_is_preserved_and_rejected(self):
        self.db.execute("UPDATE schema_meta SET version=2")
        with self.assertRaisesRegex(ValueError, "unsupported_schema_version"):
            app.initialize(self.db)
        self.assertEqual(self.db.execute("SELECT version FROM schema_meta").fetchone()[0], 2)
        self.assertFalse(self.db.in_transaction)

    def test_hostile_and_unbounded_requests_are_rejected_before_database_use(self):
        valid = {"operation": "put", "key": "key", "value": "data"}
        self.assertEqual(app.request(json.dumps(valid).encode()), valid)
        invalid = [b"x" * (app.MAX_REQUEST_BYTES + 1), b'{"operation":"init","operation":"put","key":"","value":""}']
        for altered in [dict(valid, host_path="/etc"), dict(valid, value="x" * (app.MAX_VALUE_BYTES + 1)), dict(valid, key="line\nbreak"), dict(valid, operation=[]), dict(valid, operation="exec")]:
            invalid.append(json.dumps(altered).encode())
        for raw in invalid:
            with self.subTest(raw=raw[:80]):
                with self.assertRaises(ValueError):
                    app.request(raw)


if __name__ == "__main__":
    unittest.main()
