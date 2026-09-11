#!/usr/bin/env python3
"""Project bounded, typed triage fields from a scanned diagnostics bundle.

The private archive remains the canonical evidence.  This projection is safe
to retain in a public CI status artifact: it contains identifiers, states,
timestamps, and counts, never log excerpts or request data.
"""

from __future__ import annotations

import json
import importlib.util
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
import re
import sys
from typing import Any


MAX_TRIAGE_BYTES = 64 * 1024
MAX_ATTEMPTS = 50
SAFE_VALUE = re.compile(r"^[A-Za-z0-9_.:/+-]{1,128}$")
PAIR = re.compile(r"(?P<key>[A-Za-z][A-Za-z0-9_-]{0,31})=(?P<value>[A-Za-z0-9_.:/+-]{1,128})")

COLLECTOR_SPEC = importlib.util.spec_from_file_location(
    "cooking_diagnostics_collector", Path(__file__).with_name("collect-cooking-diagnostics.py")
)
if COLLECTOR_SPEC is None or COLLECTOR_SPEC.loader is None:
    raise RuntimeError("collector validators are unavailable")
COLLECTOR = importlib.util.module_from_spec(COLLECTOR_SPEC)
COLLECTOR_SPEC.loader.exec_module(COLLECTOR)

LINEAGE_FIELDS = COLLECTOR.SNAPSHOT_FIELDS
LINEAGE_STATUS_FIELDS = COLLECTOR.SNAPSHOT_STATUS_FIELDS | {"rows"}
SOURCE_LABELS = {
    "serial", "host-journal", "runtime-log", "runtime-structured",
    "browser-summary", "test-output", "lineage", "lineage-status",
}
SAFE_STATUS = COLLECTOR.SNAPSHOT_STATUS_VALUES
TRIAGE_FIELDS = {"schema", "denial", "attempts", "snapshotStatus", "sources"}
DENIAL_FIELDS = {"denial_stage", "denial_class", "run_id"}
DENIAL_STAGES = {
    "session-authentication", "lease-authorization", "request-authorization",
    "version-loading", "decryption",
}
DENIAL_CLASSES = {
    "authentication_denied", "authority_unavailable", "authorization_denied",
    "request_denied", "persistence_failure", "secret_resolution_failure", "other_failure",
}
ATTEMPT_FIELDS = {
    "event_id", "attempt_id", "attempt_number", "attempt_run_id", "attempt_state", "attempt_created_at",
    "attempt_completed_at", "run_state", "run_outcome", "run_created_at", "run_updated_at",
    "disposition", "next_eligible_at", "terminal_at", "sampled_at",
}


def _safe_path(root: Path, relative: str) -> Path:
    if not isinstance(relative, str) or PurePosixPath(relative).is_absolute():
        raise ValueError("triage source path is unsafe")
    path = root / PurePosixPath(relative)
    if path.is_symlink() or not path.is_file() or ".." in PurePosixPath(relative).parts:
        raise ValueError("triage source path is unsafe")
    return path


def _scalar(field: str, value: Any) -> Any:
    if field == "rows":
        if type(value) is not int or not 0 <= value <= COLLECTOR.MAX_SNAPSHOT_ROWS:
            raise ValueError("triage row count is invalid")
        return value
    # The collector's snapshot-status contract explicitly permits a null
    # event_id even though lineage rows require identifiers to be UUIDs.
    if field == "event_id" and value is None:
        return None
    try:
        return COLLECTOR._safe_scalar(field, value)
    except (TypeError, ValueError) as error:
        raise ValueError(f"triage scalar is invalid: {field}") from error


def _load_lineage(path: Path) -> list[dict[str, Any]]:
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        value = json.loads(line)
        if not isinstance(value, dict) or set(value) - LINEAGE_FIELDS:
            raise ValueError("lineage contains an unknown field")
        rows.append({field: _scalar(field, item) for field, item in value.items()})
    return rows


def _latest_attempts(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    latest: dict[str, tuple[tuple[datetime, int], dict[str, Any]]] = {}
    for index, row in enumerate(rows):
        key = row.get("attempt_id") or row.get("attempt_run_id")
        if key is None:
            continue
        timestamp = next(
            (row[field] for field in ("terminal_at", "attempt_completed_at", "run_updated_at", "sampled_at", "attempt_created_at", "run_created_at") if row.get(field)),
            "1970-01-01T00:00:00Z",
        )
        normalized = timestamp.replace(" Z", "+00:00")
        if normalized.endswith("Z"):
            normalized = normalized[:-1] + "+00:00"
        parsed = datetime.fromisoformat(normalized.replace(" ", "T", 1))
        sort_key = (parsed.astimezone(timezone.utc), index)
        previous = latest.get(key)
        if previous is None or sort_key >= previous[0]:
            latest[key] = (sort_key, row)
    ordered = [item[1] for item in sorted(latest.values(), key=lambda item: item[0], reverse=True)]
    return [{field: row[field] for field in ATTEMPT_FIELDS if field in row} for row in ordered[:MAX_ATTEMPTS]]


def _load_snapshot_status(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or set(value) - LINEAGE_STATUS_FIELDS:
        raise ValueError("snapshot status contains an unknown field")
    if value.get("schema") != 1 or value.get("status") not in SAFE_STATUS:
        raise ValueError("snapshot status classification is invalid")
    result = {"schema": 1, "status": value["status"]}
    for field in ("sampled_at", "mailbox_id", "event_id"):
        if field in value:
            result[field] = _scalar(field, value[field])
    if "rows" in value:
        result["rows"] = _scalar("rows", value["rows"])
    return result


def _project_denial(
    root: Path,
    source_records: list[dict[str, Any]],
    attempts: list[dict[str, Any]],
) -> dict[str, str] | None:
    correlated_run_ids = {
        row["attempt_run_id"].lower() for row in attempts if "attempt_run_id" in row
    }
    candidates: list[dict[str, str]] = []
    for record in source_records:
        if record.get("label") not in {"serial", "runtime-log", "runtime-structured"}:
            continue
        path = _safe_path(root, record["path"])
        for line in path.read_text(encoding="utf-8").splitlines():
            fields = {match.group("key"): match.group("value") for match in PAIR.finditer(line)}
            if {"denial_stage", "denial_class", "run_id"} <= fields.keys():
                if fields["denial_stage"] not in DENIAL_STAGES or fields["denial_class"] not in DENIAL_CLASSES:
                    raise ValueError("denial classification is invalid")
                if not COLLECTOR.UUID_RE.fullmatch(fields["run_id"]):
                    raise ValueError("denial run identifier is invalid")
                if fields["run_id"].lower() not in correlated_run_ids:
                    continue
                candidates.append({
                    "denial_stage": fields["denial_stage"],
                    "denial_class": fields["denial_class"],
                    "run_id": fields["run_id"].lower(),
                })
    if not candidates:
        return None
    return candidates[-1]


def summarize(bundle: Path) -> dict[str, Any]:
    manifest_path = _safe_path(bundle, "manifest.json")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if not isinstance(manifest, dict) or manifest.get("credentialScan") != "passed":
        raise ValueError("triage requires a passed credential scan")
    records = manifest.get("sources")
    errors = manifest.get("collectionErrors")
    if not isinstance(records, list) or not isinstance(errors, list):
        raise ValueError("manifest source metadata is invalid")
    available: list[str] = []
    truncated = 0
    unavailable = 0
    for record in records:
        if not isinstance(record, dict) or set(record) - {"label", "path", "bytes", "rows", "sha256"}:
            raise ValueError("manifest source metadata contains an unknown field")
        label = record.get("label")
        if label not in SOURCE_LABELS or not isinstance(record.get("path"), str):
            raise ValueError("manifest source label is invalid")
        _safe_path(bundle, record["path"])
        available.append(label)
        source = (bundle / record["path"]).read_text(encoding="utf-8")
        # The collector intentionally projects only the status field from
        # bounded-copy records; the enclosing manifest label supplies source
        # identity without retaining the original log text.
        truncated += sum("status=truncated" in line for line in source.splitlines())
        unavailable += sum("status=unavailable" in line for line in source.splitlines())
    missing: list[str] = []
    for error in errors:
        if not isinstance(error, dict) or set(error) - {"label", "status"}:
            raise ValueError("collection error contains an unknown field")
        if error.get("label") not in SOURCE_LABELS or error.get("status") != "missing":
            raise ValueError("collection error is not a safe missing-source record")
        missing.append(error["label"])
    lineage_record = next((record for record in records if record["label"] == "lineage"), None)
    status_record = next((record for record in records if record["label"] == "lineage-status"), None)
    attempts = _latest_attempts(_load_lineage(_safe_path(bundle, lineage_record["path"]))) if lineage_record else []
    snapshot_status = _load_snapshot_status(_safe_path(bundle, status_record["path"])) if status_record else None
    result: dict[str, Any] = {
        "schema": 1,
        "denial": _project_denial(bundle, records, attempts),
        "attempts": attempts,
        "snapshotStatus": snapshot_status,
        "sources": {
            "available": sorted(set(available)),
            "missing": sorted(set(missing)),
            "availableCount": len(set(available)),
            "missingCount": len(set(missing)),
            "unavailableCount": unavailable,
            "truncatedCount": truncated,
        },
    }
    encoded = (json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n").encode()
    if len(encoded) > MAX_TRIAGE_BYTES:
        raise ValueError("triage projection exceeds retention limit")
    return result


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: summarize-cooking-diagnostics.py BUNDLE_ROOT", file=sys.stderr)
        return 2
    try:
        print(json.dumps(summarize(Path(argv[1])), sort_keys=True, separators=(",", ":")))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"triage projection failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
