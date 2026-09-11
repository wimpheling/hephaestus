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
TRIAGE_FIELDS = {
    "schema", "collectionStatus", "rejectedSources", "denial", "attempts", "snapshotStatus", "sources", "failures",
}
DENIAL_FIELDS = {"denial_stage", "denial_class", "run_id"}
DENIAL_STAGES = {
    "session-authentication", "lease-authorization", "request-authorization",
    "version-loading", "decryption",
}
DENIAL_CLASSES = {
    "authentication_denied", "authority_unavailable", "authorization_denied",
    "request_denied", "persistence_failure", "secret_resolution_failure", "other_failure",
}
REJECTION_REASONS = {
    "credential-scan-rejected", "secret-assignment-rejected", "source-policy-rejected",
    "source-limit-rejected", "source-validation-rejected",
}
ATTEMPT_FIELDS = {
    "event_id", "attempt_id", "attempt_number", "attempt_run_id", "attempt_state", "attempt_created_at",
    "attempt_completed_at", "run_state", "run_outcome", "run_created_at", "run_updated_at",
    "disposition", "next_eligible_at", "terminal_at", "sampled_at",
}
FAILURE_SOURCE_LABELS = {"serial", "host-journal", "runtime-log", "runtime-structured"}
FAILURE_MARKERS = {
    "HEPH_GCP_TEST",
    "HEPH_GCP_RUNTIME",
    "HEPH_GCP_KVM_BUILD_ERROR",
    "HEPH_GCP_KVM_FIRST_ERROR",
    "HEPH_GCP_KVM_SMOKE",
    "HEPH_GCP_RUNNER_IMAGE_READINESS",
    "HEPHAESTUS_GCP_COOKING",
}
FAILURE_FIELDS = {
    "test", "test_result", "status", "phase", "error_class", "location", "run_id",
    "attempt_run_id", "exit_code", "exit_signal", "event", "operation", "reason_class", "class", "tool",
}
FAILURE_STATUS_VALUES = {"failed", "error", "timeout", "timed-out", "nonzero"}
FAILURE_VALUE = re.compile(r"^[A-Za-z0-9_.:/+-]{1,192}$")
FAILURE_PAIR = re.compile(r"(?P<key>[A-Za-z][A-Za-z0-9_-]{0,31})=(?P<value>[A-Za-z0-9_.:/+-]{1,192})")
FAILURE_ERROR_CLASSES = {
    "permission-denied", "operation-not-permitted", "invalid-argument", "not-found",
    "connection-refused", "connection-reset", "guest-start-failed", "guest-exit",
    "worker-start-failed", "timeout", "unknown",
    "node-not-runnable", "node-version-mismatch", "rust-not-runnable", "rust-version-mismatch",
    "oras-not-runnable", "oras-version-mismatch", "chromium-missing", "chromium-not-runnable",
    "chromium-version-mismatch", "libclang-missing",
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


def _failure_record(
    source_label: str,
    fields: dict[str, Any],
    correlated_run_ids: set[str],
) -> dict[str, Any] | None:
    if not fields:
        return None
    for field in ("status", "test_result"):
        if field in fields and fields[field] not in FAILURE_STATUS_VALUES:
            fields.pop(field)
    if "error_class" in fields and fields["error_class"] not in FAILURE_ERROR_CLASSES:
        fields.pop("error_class")
    for field in ("run_id", "attempt_run_id"):
        if field in fields and (
            not isinstance(fields[field], str) or not COLLECTOR.UUID_RE.fullmatch(fields[field])
        ):
            fields.pop(field)
    if "run_id" in fields and fields["run_id"].lower() not in correlated_run_ids:
        fields.pop("run_id")
    if "attempt_run_id" in fields and fields["attempt_run_id"].lower() not in correlated_run_ids:
        fields.pop("attempt_run_id")
    exit_code = fields.get("exit_code")
    successful_exit = exit_code == 0 or exit_code == "0"
    has_failure_state = any(
        fields.get(field) in FAILURE_STATUS_VALUES for field in ("status", "test_result")
    ) or "error_class" in fields or "exit_signal" in fields
    if successful_exit and not has_failure_state:
        return None
    if not any(field in fields for field in ("test", "test_result", "status", "error_class", "exit_code", "exit_signal")):
        return None
    result: dict[str, Any] = {"source": source_label}
    for field in sorted(fields):
        value = fields[field]
        if field in {"exit_code", "exit_signal"}:
            limit = 255 if field == "exit_code" else 64
            if isinstance(value, int):
                if not 0 <= value <= limit:
                    continue
            elif isinstance(value, str) and value.isdigit() and int(value) <= limit:
                value = int(value)
            else:
                continue
        result[field] = value
    result["correlated"] = bool(
        result.get("run_id") in correlated_run_ids
        or result.get("attempt_run_id") in correlated_run_ids
    )
    return result


def _project_failures(
    root: Path,
    source_records: list[dict[str, Any]],
    attempts: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    correlated_run_ids = {
        value.lower()
        for row in attempts
        for field in ("attempt_run_id", "run_id")
        if (value := row.get(field))
    }
    failures: list[dict[str, Any]] = []
    for record in source_records:
        label = record.get("label")
        if label not in FAILURE_SOURCE_LABELS:
            continue
        path = _safe_path(root, record["path"])
        for line in path.read_text(encoding="utf-8").splitlines():
            fields: dict[str, Any] = {}
            stripped = line.strip()
            if stripped.startswith("{"):
                try:
                    value = json.loads(stripped)
                except json.JSONDecodeError:
                    continue
                if not isinstance(value, dict):
                    continue
                fields = {
                    field: item
                    for field, item in value.items()
                    if field in FAILURE_FIELDS and isinstance(item, (str, int))
                }
                if fields.get("test_result") not in FAILURE_STATUS_VALUES and fields.get("status") not in FAILURE_STATUS_VALUES:
                    continue
            else:
                marker = stripped.split(maxsplit=1)[0].rstrip(":") if stripped else ""
                if marker not in FAILURE_MARKERS:
                    continue
                fields = {
                    match.group("key"): match.group("value")
                    for match in FAILURE_PAIR.finditer(stripped)
                    if match.group("key") in FAILURE_FIELDS
                }
                if "class" in fields:
                    fields["error_class"] = fields.pop("class")
                if "tool" in fields:
                    fields["test"] = fields.pop("tool")
                if fields.get("status", "").isdigit():
                    fields["exit_code"] = fields.pop("status")
                if "error=" in stripped and "error_class" not in fields:
                    error_value = dict(FAILURE_PAIR.findall(stripped)).get("error")
                    if error_value in FAILURE_ERROR_CLASSES:
                        fields["error_class"] = error_value
                if not any(field in fields for field in ("test", "status", "error_class", "exit_code", "exit_signal")):
                    continue
            fields = {
                field: value
                for field, value in fields.items()
                if field in FAILURE_FIELDS
                and (isinstance(value, int) or (isinstance(value, str) and FAILURE_VALUE.fullmatch(value)))
            }
            projected = _failure_record(label, fields, correlated_run_ids)
            if projected is not None and projected not in failures:
                failures.append(projected)
            if len(failures) >= 50:
                return failures
    return failures


def summarize(bundle: Path) -> dict[str, Any]:
    manifest_path = _safe_path(bundle, "manifest.json")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if not isinstance(manifest, dict) or manifest.get("credentialScan") != "passed":
        raise ValueError("triage requires a passed credential scan")
    records = manifest.get("sources")
    errors = manifest.get("collectionErrors")
    collection_status = manifest.get("collectionStatus")
    rejected_sources = manifest.get("rejectedSources")
    if collection_status not in {"complete", "partial"} or not isinstance(rejected_sources, list):
        raise ValueError("manifest collection status is invalid")
    if not isinstance(records, list) or not isinstance(errors, list):
        raise ValueError("manifest source metadata is invalid")
    for rejected in rejected_sources:
        if (
            not isinstance(rejected, dict)
            or set(rejected) != {"label", "reason", "status"}
            or rejected.get("label") not in SOURCE_LABELS
            or rejected.get("reason") not in REJECTION_REASONS
            or rejected.get("status") != "rejected"
        ):
            raise ValueError("manifest rejected source metadata is invalid")
    if collection_status == "complete" and (errors or rejected_sources):
        raise ValueError("complete collection contains partial source metadata")
    if collection_status == "partial" and not (errors or rejected_sources):
        raise ValueError("partial collection has no source error metadata")
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
        "collectionStatus": collection_status,
        "rejectedSources": rejected_sources,
        "denial": _project_denial(bundle, records, attempts),
        "attempts": attempts,
        "snapshotStatus": snapshot_status,
        "failures": _project_failures(bundle, records, attempts),
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
