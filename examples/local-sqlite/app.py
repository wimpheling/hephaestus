#!/usr/local/bin/python3
"""Bounded SQLite workload; Heph authorizes its mounted bytes, not SQL queries."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import sys
import time

MAX_REQUEST_BYTES = 8192
MAX_RESPONSE_BYTES = 32768
MAX_ROWS = 256
MAX_VALUE_BYTES = 4096
SCHEMA_VERSION = 1
OPERATIONS = {"init", "put", "get", "inspect", "integrity", "backup"}


def strict_object(pairs):
    result = {}
    for name, value in pairs:
        if name in result:
            raise ValueError("duplicate_request_field")
        result[name] = value
    return result


def request(raw):
    if len(raw) > MAX_REQUEST_BYTES:
        raise ValueError("request_too_large")
    value = json.loads(raw, object_pairs_hook=strict_object)
    if not isinstance(value, dict) or set(value) != {"operation", "key", "value"}:
        raise ValueError("invalid_request_fields")
    if not isinstance(value["operation"], str) or value["operation"] not in OPERATIONS:
        raise ValueError("unsupported_operation")
    for name, maximum in [("key", 128), ("value", MAX_VALUE_BYTES)]:
        text = value[name]
        if not isinstance(text, str) or len(text.encode("utf-8")) > maximum:
            raise ValueError("invalid_" + name)
        if any(ord(character) < 32 for character in text):
            raise ValueError("invalid_" + name)
    if value["operation"] in {"put", "get"} and not value["key"]:
        raise ValueError("empty_key")
    if value["operation"] != "put" and value["value"]:
        raise ValueError("unused_value")
    if value["operation"] not in {"put", "get"} and value["key"]:
        raise ValueError("unused_key")
    return value


def connect(path):
    connection = sqlite3.connect(path, timeout=5, isolation_level=None)
    connection.execute("PRAGMA page_size=4096")
    connection.execute("PRAGMA max_page_count=1024")
    connection.execute("PRAGMA cache_size=-1024")
    connection.execute("PRAGMA journal_mode=WAL")
    connection.execute("PRAGMA synchronous=FULL")
    connection.execute("PRAGMA journal_size_limit=4194304")
    connection.execute("PRAGMA wal_autocheckpoint=128")
    return connection


def initialize(connection):
    """An interrupted schema transaction cannot advance the version alone."""
    connection.execute("BEGIN IMMEDIATE")
    try:
        connection.execute("CREATE TABLE IF NOT EXISTS schema_meta(singleton INTEGER PRIMARY KEY CHECK(singleton=1), version INTEGER NOT NULL)")
        row = connection.execute("SELECT version FROM schema_meta WHERE singleton=1").fetchone()
        if row is None:
            connection.execute("CREATE TABLE entries(key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            connection.execute("INSERT INTO schema_meta(singleton,version) VALUES(1,?)", (SCHEMA_VERSION,))
        elif row[0] != SCHEMA_VERSION:
            raise ValueError("unsupported_schema_version")
        connection.commit()
    except BaseException:
        connection.rollback()
        raise


def put(connection, key, value):
    connection.execute("BEGIN IMMEDIATE")
    try:
        exists = connection.execute("SELECT 1 FROM entries WHERE key=?", (key,)).fetchone()
        count = connection.execute("SELECT count(*) FROM entries").fetchone()[0]
        if exists is None and count >= MAX_ROWS:
            raise ValueError("entry_limit")
        connection.execute("INSERT INTO entries(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (key, value))
        connection.commit()
    except BaseException:
        connection.rollback()
        raise


def integrity(connection):
    rows = connection.execute("PRAGMA integrity_check").fetchmany(2)
    if rows != [("ok",)]:
        raise ValueError("database_integrity_failed")
    return "ok"


def prepare_backup(connection, destination):
    """SQLite copies committed data, including WAL; never copy DB files live."""
    descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.close(descriptor)
    target = sqlite3.connect(destination, isolation_level=None)
    try:
        # Both operations are workload-owned SQLite connections. Existing backup
        # artifacts are retained; operators must collect them before another backup.
        deadline = time.monotonic() + 5
        def progress(_status, _remaining, _total):
            if time.monotonic() > deadline:
                raise TimeoutError("backup_deadline")
        connection.backup(target, pages=32, sleep=0.01, progress=progress)
        integrity(target)
        rows = target.execute("SELECT count(*) FROM entries").fetchone()[0]
        target.close()
        with open(destination, "rb") as backing:
            os.fsync(backing.fileno())
            checksum = hashlib.file_digest(backing, "sha256").hexdigest()
        directory = os.open(Path(destination).parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        return {"artifact": "backup.sqlite", "sha256": checksum, "rows": rows}
    except BaseException:
        target.close()
        # Incomplete bytes remain visible; a retry must not overwrite them.
        raise


def execute(connection, command, backup_path):
    initialize(connection)
    operation = command["operation"]
    if operation == "put":
        put(connection, command["key"], command["value"])
        return {"written": True}
    if operation == "get":
        row = connection.execute("SELECT value FROM entries WHERE key=?", (command["key"],)).fetchone()
        return {"value": row[0] if row else None}
    if operation == "backup":
        return prepare_backup(connection, backup_path)
    if operation == "integrity":
        return {"integrity": integrity(connection)}
    return {"schema_version": SCHEMA_VERSION, "rows": connection.execute("SELECT count(*) FROM entries").fetchone()[0]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stdin", action="store_true", help="read bounded development request instead of runtime parameters")
    arguments = parser.parse_args()
    try:
        if arguments.stdin:
            raw = sys.stdin.buffer.read(MAX_REQUEST_BYTES + 1)
        else:
            with open("/run/hephaestus/parameters.json", "rb") as parameters:
                raw = parameters.read(MAX_REQUEST_BYTES + 1)
        command = request(raw)
        # The provider controls this guest mount; no request supplies a path.
        connection = connect("/data/app.sqlite")
        try:
            result = execute(connection, command, "/data/backup.sqlite")
        finally:
            connection.close()
        encoded = json.dumps(result, sort_keys=True).encode("utf-8")
        if len(encoded) > MAX_RESPONSE_BYTES:
            raise ValueError("response_too_large")
        sys.stdout.buffer.write(encoded + b"\n")
        return 0
    except (ValueError, sqlite3.Error, OSError, TypeError):
        # Provider paths and database contents are not included in diagnostics.
        print('{"error":"workload_operation_failed"}', file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
