#!/usr/bin/env python3
"""Sanitize bounded PostgreSQL lifecycle snapshot rows before retention.

The SQL snapshot represents VM log chunks as JSON byte arrays.  Decode only a
bounded chunk, pass it through the shared streaming fixture-credential
redactor, and retain text without the recoverable byte array.  Metadata rows
are intentionally limited to identifiers and lifecycle classifications by the
caller.
"""

from __future__ import annotations

import importlib.util
import io
import json
import sys
from pathlib import Path
from collections.abc import Iterable


MAX_LINE_BYTES = 256 * 1024
MAX_LOG_BYTES = 64 * 1024
MAX_TOTAL_LOG_BYTES = 2 * 1024 * 1024


def _load_evidence_module():
    path = Path(__file__).with_name("check-browser-evidence.py")
    spec = importlib.util.spec_from_file_location("browser_evidence", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("shared evidence redactor is unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


EVIDENCE = _load_evidence_module()


def _decode_vm_log(row: dict[str, object]) -> tuple[tuple[str, str], int, bytes] | None:
    if row.get("surface") != "run_vm_log":
        return None
    run_id = row.get("run_id")
    stream = row.get("stream")
    if not isinstance(run_id, str) or not isinstance(stream, str):
        raise ValueError("run VM log identity is invalid")
    sequence = row.get("sequence")
    if type(sequence) is not int or sequence < 1:  # bool is not a byte sequence number
        raise ValueError("run VM log sequence is invalid")
    encoded = row.get("bytes")
    if not isinstance(encoded, list) or len(encoded) > MAX_LOG_BYTES:
        raise ValueError("run VM log byte array exceeds the retention limit")
    values: list[int] = []
    for value in encoded:
        if type(value) is not int or not 0 <= value <= 255:
            raise ValueError("run VM log contains an invalid byte")
        values.append(value)
    return (run_id, stream), sequence, bytes(values)


def _redact_group(
    key: tuple[str, str], rows: list[tuple[dict[str, object], int, bytes]]
) -> dict[str, object]:
    source = io.BytesIO(b"".join(chunk for _, _, chunk in rows))
    output = io.BytesIO()
    matched = EVIDENCE.redact_stream(source, output)
    metadata = dict(rows[0][0])
    metadata.pop("bytes", None)
    metadata.pop("sequence", None)
    metadata.update(
        {
            "surface": "run_vm_log",
            "run_id": key[0],
            "stream": key[1],
            "sequences": [sequence for _, sequence, _ in rows],
            "bytes_retained": sum(len(chunk) for _, _, chunk in rows),
            "credential_redacted": matched,
            "text": output.getvalue().decode("utf-8", errors="replace"),
        }
    )
    return metadata


def sanitize_row(row: dict[str, object]) -> dict[str, object]:
    """Return one safe row, rejecting malformed or over-sized VM evidence."""

    decoded = _decode_vm_log(row)
    if decoded is None:
        return row
    key, sequence, data = decoded
    return _redact_group(key, [(row, sequence, data)])


class SnapshotStream:
    """Group only contiguous rows for one run/stream before redaction."""

    def __init__(self) -> None:
        self.total_log_bytes = 0
        self.key: tuple[str, str] | None = None
        self.rows: list[tuple[dict[str, object], int, bytes]] = []
        self.group_bytes = 0

    def _flush(self) -> list[dict[str, object]]:
        if not self.rows or self.key is None:
            return []
        result = [_redact_group(self.key, self.rows)]
        self.key = None
        self.rows = []
        self.group_bytes = 0
        return result

    def accept(self, row: dict[str, object]) -> list[dict[str, object]]:
        decoded = _decode_vm_log(row)
        if decoded is None:
            return self._flush() + [row]
        key, sequence, data = decoded
        self.total_log_bytes += len(data)
        if self.total_log_bytes > MAX_TOTAL_LOG_BYTES:
            raise ValueError("run VM log evidence exceeds the retention limit")
        result: list[dict[str, object]] = []
        if self.key != key:
            result.extend(self._flush())
            self.key = key
        self.group_bytes += len(data)
        if self.group_bytes > MAX_TOTAL_LOG_BYTES:
            raise ValueError("run VM log group exceeds the retention limit")
        self.rows.append((row, sequence, data))
        return result

    def finish(self) -> list[dict[str, object]]:
        return self._flush()


def main() -> int:
    stream = SnapshotStream()
    while True:
        raw = sys.stdin.buffer.readline(MAX_LINE_BYTES + 1)
        if not raw:
            break
        if len(raw) > MAX_LINE_BYTES:
            raise ValueError("snapshot row exceeds the retention limit")
        row = json.loads(raw)
        if not isinstance(row, dict):
            raise ValueError("snapshot row is not an object")
        for safe in stream.accept(row):
            sys.stdout.write(json.dumps(safe, sort_keys=True, separators=(",", ":")) + "\n")
            sys.stdout.flush()
    for safe in stream.finish():
        sys.stdout.write(json.dumps(safe, sort_keys=True, separators=(",", ":")) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, TypeError, json.JSONDecodeError):
        # Keep malformed-input diagnostics fixed and payload-free.  The
        # caller removes any partially retained file on a nonzero status.
        print("run snapshot sanitization failed", file=sys.stderr)
        raise SystemExit(1)
