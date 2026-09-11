#!/usr/bin/env python3
"""Retain a bounded, redacted Cooking failure/success evidence bundle.

Inputs are explicit files supplied by the disposable test harness.  The
collector deliberately does not walk log directories or accept browser trace,
HAR, screenshot, cookie, or request/response files.  A lineage JSONL input is
validated against a metadata-only schema before it is retained.  All retained
files are scanned before an optional archive is created.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePath
import re
import shutil
import sys
import tarfile
from typing import Any


MAX_SOURCE_BYTES = 16 * 1024 * 1024
MAX_TOTAL_BYTES = 128 * 1024 * 1024
MAX_SNAPSHOT_BYTES = 8 * 1024 * 1024
MAX_SNAPSHOT_ROWS = 20_000
MAX_LINE_BYTES = 256 * 1024
LABEL_RE = re.compile(r"^[a-z][a-z0-9_-]{0,47}$")
STATUS_RE = re.compile(r"^[a-z][a-z0-9_:-]{0,63}$")
TIMESTAMP_RE = re.compile(
    r"^\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z| Z| [+-]\d{2}:\d{2}(?::\d{2})?)$"
)
UUID_RE = re.compile(
    r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
)
ALLOWED_LABELS = frozenset(
    {
        "serial",
        "host-journal",
        "runtime-log",
        "runtime-structured",
        "browser-summary",
        "test-output",
        # These labels are used only in collectionErrors for producer files
        # that were absent; they are never accepted as retained raw sources.
        "lineage",
        "lineage-status",
    }
)
FORBIDDEN_BASENAME = re.compile(
    r"(?:trace|har|storage|cookie|screenshot|request|response|network|credential|secret)",
    re.IGNORECASE,
)
SNAPSHOT_FIELDS = frozenset(
    {
        "sampled_at",
        "mailbox_id",
        "event_id",
        "attempt_id",
        "attempt_number",
        "attempt_run_id",
        "attempt_state",
        "attempt_created_at",
        "attempt_completed_at",
        "run_state",
        "run_outcome",
        "run_created_at",
        "run_updated_at",
        "disposition",
        "next_eligible_at",
        "terminal_at",
    }
)
SNAPSHOT_STATUS_FIELDS = frozenset(
    {"schema", "status", "sampled_at", "mailbox_id", "event_id", "rows"}
)
ID_FIELDS = frozenset(
    {
        "mailbox_id",
        "event_id",
        "attempt_id",
        "attempt_run_id",
    }
)
STATUS_FIELDS = frozenset(
    {
        "attempt_state",
        "run_state",
        "run_outcome",
        "disposition",
    }
)
NULLABLE_FIELDS = frozenset(
    {"attempt_completed_at", "run_outcome", "next_eligible_at", "terminal_at"}
)
STATUS_VALUES = {
    "attempt_state": frozenset({"leased", "running", "completed", "failed", "uncertain"}),
    "run_state": frozenset(
        {
            "queued",
            "leasing_volume",
            "provisioning",
            "starting",
            "running",
            "succeeded",
            "failed",
            "cancelled",
            "cleaning_up",
            "cleaned_up",
        }
    ),
    "run_outcome": frozenset({"succeeded", "failed", "cancelled"}),
    "disposition": frozenset(
        {"pending", "eligible", "leased", "running", "delivered", "denied", "dead_lettered", "cancelled", "retryable"}
    ),
}
SNAPSHOT_STATUS_VALUES = frozenset({"ok", "query_timeout", "query_failed", "write_failed"})
DROP_LINE_RE = re.compile(
    r"(?i)(?:authorization\s*[:=]|bearer\s+|x-telegram-bot-api-secret-token\s*[:=]|"
    r"(?:request|response|http)[_-]?(?:body|headers?)\s*[:=]|"
    r"[\"']?(?:request|response|body|headers?|json|data)[\"']?\s*[:=]|"
    r"(?:cookie|storage(?:state)?|payload|environment|env|credential|password|token|secret)\s*[:=])"
)
PAIR_RE = re.compile(
    r"(?P<key>[A-Za-z][A-Za-z0-9_-]{0,31})\s*=\s*[\"']?(?P<value>[^\s\"']+)[\"']?"
)
SAFE_KEYS = frozenset(
    {
        "event",
        "phase",
        "revision",
        "mode",
        "status",
        "exit",
        "rc",
        "slot",
        "error_class",
        "denial_stage",
        "denial_class",
        "run_id",
        "attempt",
        "attempt_number",
        "state",
        "outcome",
        "disposition",
        "test",
        "suite",
        "duration_ms",
        "unit",
        "operation",
        "operation_id",
        "pid",
        "exit_code",
        "exit_signal",
        "reason_class",
        "test_result",
        "diagnostics_result",
        "diagnostic_probe_completed",
        "expected_fixture",
        "collection_status",
        "upload_status",
        "assertion",
        "error",
        "location",
        "timestamp",
    }
)
SAFE_VALUE_RE = re.compile(r"^[A-Za-z0-9_.:/+-]{1,128}$")
GENERIC_SECRET_RE = re.compile(
    rb"(?i)\b(?:password|secret|token|api[_-]?key|private[_-]?key|authorization)\s*[:=]\s*(?!\[REDACTED\])[^\s,;}]+"
)
SAFE_BARE_RE = re.compile(r"^(?:PASS|FAIL|ERROR|WARN|FAILED|READY|TIMEOUT)$", re.IGNORECASE)
SAFE_STACK_RE = re.compile(r"^at\s+[A-Za-z0-9_.:/-]+:\d+(?::\d+)?$")
SAFE_ERROR_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9 _.,:;/'()\[\]-]{0,1023}$")
STACK_LINE_RE = re.compile(r"^\s*(?:at\s+|File\s+|[A-Za-z0-9_.-]+\.\w+:\d+)")
ASSERTION_LINE_RE = re.compile(
    r"^(?:Assertion(?:Error)?|assertion(?: failed)?|FAIL(?:ED)?):\s*(?P<text>.+)$",
    re.IGNORECASE,
)
ERROR_LINE_RE = re.compile(r"^(?:ERROR|WARN):\s*(?P<text>.+)$", re.IGNORECASE)
BROWSER_DROP_WORD_RE = re.compile(
    r"(?i)\b(?:request|response|body|headers?|cookie|storage|payload|secret|token|authorization)\b"
)
ANSI_RE = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")
RUNTIME_MARKER_RE = re.compile(
    r"\b(HEPH(?:AESTUS)?_[A-Z0-9_:-]+|secret[_ -]?broker|broker[_ -]?https|"
    r"vm[_ -]?libkrun|libkrun|worker|passt|cooking|runtime)\b",
    re.IGNORECASE,
)
RUNTIME_TIMESTAMP_RE = re.compile(
    r"\b20\d{2}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z\b"
)
RUNTIME_LIFECYCLE_EVENTS = (
    ("provisioning VM resources", "provision"),
    ("starting libkrun worker", "start"),
    ("microVM started", "start"),
    ("guest ready", "guestready"),
    ("worker reaped", "reap"),
    ("VM resources cleaned", "cleanup"),
)
RUNTIME_FIELDS = frozenset(
    {
        "test_result",
        "diagnostics_result",
        "phase",
        "revision",
        "status",
        "mode",
        "exit",
        "exit_code",
        "exit_signal",
        "event",
        "run_id",
        "operation",
        "operation_id",
        "pid",
        "reason_class",
        "collection_status",
        "upload_status",
        "diagnostic_probe_completed",
        "expected_fixture",
        "timestamp",
    }
)
READINESS_ERRORS = {
    "custom runner image Node executable cannot run as forge": ("node", "node-not-runnable"),
    "custom runner image Node executable version does not match its pin": ("node", "node-version-mismatch"),
    "custom runner image Rust executable cannot run as forge": ("rust", "rust-not-runnable"),
    "custom runner image Rust executable version does not match its pin": ("rust", "rust-version-mismatch"),
    "custom runner image ORAS executable cannot run as forge": ("oras", "oras-not-runnable"),
    "custom runner image ORAS executable version does not match its pin": ("oras", "oras-version-mismatch"),
    "custom runner image Chromium executable is missing": ("chromium", "chromium-missing"),
    "custom runner image Chromium executable cannot run as forge": ("chromium", "chromium-not-runnable"),
    "custom runner image Chromium executable version does not match its manifest": (
        "chromium",
        "chromium-version-mismatch",
    ),
    "baked runner image has no usable libclang shared library": ("libclang", "libclang-missing"),
    "libclang shared library is unavailable": ("libclang", "libclang-missing"),
}


def classify_readiness_error(line: str) -> str | None:
    """Map one exact startup readiness error to a safe typed marker."""

    normalized = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", line).strip()
    if "startup-script:" in normalized:
        normalized = normalized.split("startup-script:", 1)[1].lstrip()
    prefix = "gcp-kvm-startup: "
    if not normalized.startswith(prefix):
        return None
    message = normalized[len(prefix) :]
    value = READINESS_ERRORS.get(message)
    if value is None:
        return None
    tool, error_class = value
    return f"HEPH_GCP_RUNNER_IMAGE_READINESS tool={tool} class={error_class} phase=runner-image-runtime"


BROWSER_FIELDS = frozenset(
    {"status", "suite", "test", "phase", "duration_ms", "exit_code", "error", "stack", "started_at", "finished_at"}
)


def _load_evidence_module():
    path = Path(__file__).with_name("check-browser-evidence.py")
    spec = importlib.util.spec_from_file_location("browser_evidence", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("shared evidence scanner is unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


EVIDENCE = _load_evidence_module()


class CollectionError(ValueError):
    """An input failed the collector's bounded safe-retention contract."""


def _reject_secret_assignments(data: bytes) -> None:
    if GENERIC_SECRET_RE.search(data):
        raise CollectionError("source contains an unredacted secret assignment")


def _safe_input(path: Path) -> Path:
    if not path.is_absolute() or path.is_symlink() or not path.is_file() or _has_symlink_ancestor(path):
        raise CollectionError(f"source must be an absolute regular non-symlink file: {path}")
    if FORBIDDEN_BASENAME.search(path.name):
        raise CollectionError(f"raw browser or credential-bearing source is forbidden: {path.name}")
    try:
        size = path.stat(follow_symlinks=False).st_size
    except OSError as error:
        raise CollectionError("source metadata is unavailable") from error
    if size > MAX_SOURCE_BYTES:
        raise CollectionError(f"source exceeds {MAX_SOURCE_BYTES} bytes: {path.name}")
    return path


def _has_symlink_ancestor(path: Path) -> bool:
    current = Path(path.anchor)
    for part in path.parts[1:-1]:
        current /= part
        if current.is_symlink():
            return True
    return False


def _open_safe(path: Path):
    """Open a validated input without allowing a final symlink race."""

    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        return os.fdopen(os.open(_safe_input(path), flags), "rb", closefd=True)
    except OSError as error:
        raise CollectionError("source cannot be opened safely") from error


def _safe_label(value: str) -> str:
    if not LABEL_RE.fullmatch(value) or value not in ALLOWED_LABELS:
        raise CollectionError(f"source label is not allowlisted: {value}")
    return value


def _parse_source(value: str) -> tuple[str, Path]:
    label, separator, raw_path = value.partition("=")
    if not separator or not raw_path:
        raise CollectionError("source must use LABEL=ABSOLUTE_PATH")
    label = _safe_label(label)
    if label in {"lineage", "lineage-status"}:
        raise CollectionError("lineage labels are reserved for --missing-source")
    return label, _safe_input(Path(raw_path))


def _safe_scalar(field: str, value: Any) -> Any:
    if value is None:
        if field in NULLABLE_FIELDS:
            return None
        raise CollectionError(f"snapshot field cannot be null: {field}")
    if field in ID_FIELDS:
        if not isinstance(value, str) or not UUID_RE.fullmatch(value):
            raise CollectionError(f"snapshot identifier is invalid: {field}")
        return value.lower()
    if field == "attempt_number":
        if type(value) is not int or not 1 <= value <= 100:
            raise CollectionError("snapshot attempt number is invalid")
        return value
    if field in STATUS_FIELDS:
        if not isinstance(value, str) or not STATUS_RE.fullmatch(value):
            raise CollectionError(f"snapshot status is invalid: {field}")
        if value not in STATUS_VALUES[field]:
            raise CollectionError(f"snapshot status is not recognized: {field}")
        return value
    if not isinstance(value, str) or not TIMESTAMP_RE.fullmatch(value) or "\x00" in value:
        raise CollectionError(f"snapshot timestamp is invalid: {field}")
    return value


def _project_runtime_fields(line: str) -> tuple[str, list[str]]:
    """Extract approved fields from tracing lifecycle lines.

    The Rust tracing formatter uses human-readable operation text and fields
    such as ``vm_id``/``worker_pid``/``signal=Some(9)``.  Normalize those
    names and values into the collector's stable, metadata-only vocabulary.
    """

    marker = RUNTIME_MARKER_RE.search(line)
    if marker is None:
        return "", []
    fields: dict[str, str] = {}
    timestamp = RUNTIME_TIMESTAMP_RE.search(line)
    if timestamp is not None:
        fields["timestamp"] = timestamp.group(0)
    for match in PAIR_RE.finditer(line):
        key = match.group("key")
        value = match.group("value")
        if key == "vm_id":
            if SAFE_VALUE_RE.fullmatch(value):
                fields.setdefault("run_id", value)
        elif key in {"worker_pid", "vmm_pid", "pid"}:
            if value.isdigit() and 0 < int(value) <= 2**31 - 1:
                fields.setdefault("pid", value)
        elif key in {"code", "signal"}:
            match_status = re.fullmatch(r"Some\((\d+)\)", value)
            if match_status is not None:
                fields["exit_code" if key == "code" else "exit_signal"] = match_status.group(1)
        elif key in SAFE_KEYS and SAFE_VALUE_RE.fullmatch(value):
            fields.setdefault(key, value)
    for phrase, event in RUNTIME_LIFECYCLE_EVENTS:
        if phrase in line:
            fields.setdefault("event", event)
            fields.setdefault("operation", event)
            break
    bare = [word.rstrip(":") for word in line.split() if SAFE_BARE_RE.fullmatch(word.rstrip(":"))]
    projected = marker.group(1) + " " + " ".join(bare + [f"{key}={value}" for key, value in fields.items()])
    return projected.strip(), list(fields)


def _project_text(source: Path, destination: Path) -> tuple[int, str]:
    """Project lifecycle key/value fields and safe assertion/stack lines."""

    source = _safe_input(source)
    retained = 0
    total = 0
    digest = hashlib.sha256()
    with _open_safe(source) as input_file, destination.open("wb") as output:
        for raw in input_file:
            total += len(raw)
            if total > MAX_SOURCE_BYTES:
                raise CollectionError(f"source exceeds {MAX_SOURCE_BYTES} bytes: {source.name}")
            EVIDENCE.check_bytes(raw, str(source))
            _reject_secret_assignments(raw)
            if len(raw) > MAX_LINE_BYTES:
                continue
            line = ANSI_RE.sub(b"", raw).decode("utf-8", errors="replace").rstrip("\r\n")
            readiness = classify_readiness_error(line)
            if readiness is not None:
                projected = readiness
            elif DROP_LINE_RE.search(line):
                continue
            else:
                marker = RUNTIME_MARKER_RE.search(line)
                stripped = line.strip()
                stack_match = SAFE_STACK_RE.fullmatch(stripped)
                assertion_match = ASSERTION_LINE_RE.fullmatch(stripped)
                error_match = ERROR_LINE_RE.fullmatch(stripped)
                if stack_match:
                    # Keep only a canonical source location; never retain the
                    # caller's free-form stack text.
                    projected = f"location={stripped[3:].strip()}"
                elif assertion_match or error_match:
                    match = assertion_match or error_match
                    assert match is not None
                    text = match.group("text").strip()
                    if (
                        not SAFE_ERROR_RE.fullmatch(text)
                        or BROWSER_DROP_WORD_RE.search(text)
                        or len(text) > 1024
                    ):
                        continue
                    field = "assertion" if assertion_match else "error"
                    projected = f"{field}={text}"
                elif marker is not None:
                    projected, fields = _project_runtime_fields(line)
                    if not fields and not projected.split()[1:]:
                        continue
                else:
                    continue
            encoded = (projected[:16 * 1024] + (" [TRUNCATED]" if len(projected) > 16 * 1024 else "") + "\n").encode()
            output.write(encoded)
            digest.update(encoded)
            retained += len(encoded)
            if retained > MAX_SOURCE_BYTES:
                raise CollectionError("projected source exceeds retention limit")
        output.flush()
        os.fsync(output.fileno())
    return retained, digest.hexdigest()


def _project_browser_summary(source: Path, destination: Path) -> tuple[int, str]:
    source = _safe_input(source)
    try:
        with _open_safe(source) as input_file:
            raw = input_file.read()
            EVIDENCE.check_bytes(raw, str(source))
            _reject_secret_assignments(raw)
            value = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise CollectionError("browser summary must be one JSON object") from error
    if not isinstance(value, dict) or set(value) - BROWSER_FIELDS:
        raise CollectionError("browser summary contains an unallowlisted field")
    safe: dict[str, Any] = {}
    for field, item in value.items():
        if item is None and field not in {"error", "stack"}:
            raise CollectionError(f"browser summary field is invalid: {field}")
        if field in {"status", "phase"}:
            if not isinstance(item, str) or item not in {"passed", "failed", "timed_out", "browser", "cooking"}:
                raise CollectionError(f"browser summary status is invalid: {field}")
        elif field in {"duration_ms", "exit_code"}:
            if type(item) is not int or item < 0:
                raise CollectionError(f"browser summary number is invalid: {field}")
        elif field in {"error", "stack"}:
            if item is None:
                safe[field] = None
                continue
            if not isinstance(item, str) or len(item) > 16 * 1024:
                raise CollectionError(f"browser summary text is invalid: {field}")
            retained_lines = []
            for line in item.splitlines():
                line = line.strip()
                if not line or BROWSER_DROP_WORD_RE.search(line):
                    continue
                if field == "stack":
                    if STACK_LINE_RE.match(line):
                        retained_lines.append(line[:1024])
                elif SAFE_ERROR_RE.fullmatch(line):
                    retained_lines.append(line[:1024])
            safe[field] = "\n".join(retained_lines)
            continue
        elif not isinstance(item, (str, type(None))) or len(item or "") > 16 * 1024:
            raise CollectionError(f"browser summary text is invalid: {field}")
        safe[field] = item
    encoded = (json.dumps(safe, sort_keys=True, separators=(",", ":")) + "\n").encode()
    destination.write_bytes(encoded)
    destination.chmod(0o600)
    return len(encoded), hashlib.sha256(encoded).hexdigest()


def _project_runtime_structured(source: Path, destination: Path) -> tuple[int, str]:
    """Retain only scalar lifecycle fields from startup JSON records."""

    source = _safe_input(source)
    with _open_safe(source) as input_file:
        raw = input_file.read(MAX_SOURCE_BYTES + 1)
    if len(raw) > MAX_SOURCE_BYTES:
        raise CollectionError(f"source exceeds {MAX_SOURCE_BYTES} bytes: {source.name}")
    if not raw.lstrip().startswith(b"{"):
        return _project_text(source, destination)
    records: list[bytes] = []
    for line in raw.splitlines():
        if not line.strip():
            continue
        EVIDENCE.check_bytes(line, str(source))
        _reject_secret_assignments(line)
        try:
            value = json.loads(line)
        except (json.JSONDecodeError, UnicodeDecodeError) as error:
            raise CollectionError("runtime structured log contains invalid JSON") from error
        if not isinstance(value, dict):
            raise CollectionError("runtime structured log record is not an object")
        safe: dict[str, Any] = {}
        for field, item in value.items():
            if field not in RUNTIME_FIELDS:
                continue
            if isinstance(item, bool):
                safe[field] = item
            elif type(item) is int and 0 <= item <= 2**31 - 1:
                safe[field] = item
            elif isinstance(item, str) and SAFE_VALUE_RE.fullmatch(item):
                safe[field] = item
            else:
                raise CollectionError(f"runtime structured field is invalid: {field}")
        if safe:
            records.append(json.dumps(safe, sort_keys=True, separators=(",", ":")).encode())
    encoded = b"\n".join(records) + (b"\n" if records else b"")
    destination.write_bytes(encoded)
    destination.chmod(0o600)
    return len(encoded), hashlib.sha256(encoded).hexdigest()


def _canonical_snapshot(source: Path, destination: Path) -> tuple[int, str]:
    source = _safe_input(source)
    if source.stat(follow_symlinks=False).st_size > MAX_SNAPSHOT_BYTES:
        raise CollectionError("lineage snapshot exceeds its retention limit")
    rows = 0
    with _open_safe(source) as input_file, destination.open("wb") as output:
        for raw in input_file:
            rows += 1
            if rows > MAX_SNAPSHOT_ROWS or len(raw) > MAX_LINE_BYTES:
                raise CollectionError("lineage snapshot row budget exceeded")
            try:
                value = json.loads(raw)
            except (json.JSONDecodeError, UnicodeDecodeError) as error:
                raise CollectionError("lineage snapshot contains invalid JSON") from error
            if not isinstance(value, dict) or set(value) - SNAPSHOT_FIELDS:
                raise CollectionError("lineage snapshot contains an unallowlisted field")
            safe = {field: _safe_scalar(field, item) for field, item in value.items()}
            output.write(json.dumps(safe, sort_keys=True, separators=(",", ":")).encode() + b"\n")
        output.flush()
        os.fsync(output.fileno())
    return rows, _sha256(destination)


def _canonical_snapshot_status(source: Path, destination: Path) -> tuple[int, str]:
    source = _safe_input(source)
    if source.stat(follow_symlinks=False).st_size > MAX_LINE_BYTES:
        raise CollectionError("lineage status exceeds its retention limit")
    try:
        with _open_safe(source) as input_file:
            raw = input_file.read(MAX_LINE_BYTES + 1)
        if len(raw) > MAX_LINE_BYTES:
            raise CollectionError("lineage status exceeds its retention limit")
        value = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise CollectionError("lineage status must be one JSON object") from error
    if not isinstance(value, dict) or set(value) - SNAPSHOT_STATUS_FIELDS:
        raise CollectionError("lineage status contains an unallowlisted field")
    if value.get("schema") != 1 or value.get("status") not in SNAPSHOT_STATUS_VALUES:
        raise CollectionError("lineage status classification is invalid")
    safe: dict[str, Any] = {"schema": 1, "status": value["status"]}
    for field in ("sampled_at", "mailbox_id", "event_id"):
        if field in value:
            if field == "event_id" and value[field] is None:
                safe[field] = None
            else:
                safe[field] = _safe_scalar(field, value[field])
    if "rows" in value:
        rows = value["rows"]
        if type(rows) is not int or not 0 <= rows <= MAX_SNAPSHOT_ROWS:
            raise CollectionError("lineage status row count is invalid")
        safe["rows"] = rows
    encoded = (json.dumps(safe, sort_keys=True, separators=(",", ":")) + "\n").encode()
    destination.write_bytes(encoded)
    destination.chmod(0o600)
    return len(encoded), hashlib.sha256(encoded).hexdigest()


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as input_file:
        for chunk in iter(lambda: input_file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _source_rejection_reason(error: Exception) -> str:
    """Map source failures to non-sensitive, stable manifest classifications."""

    message = str(error).lower()
    if "fixture credential" in message:
        return "credential-scan-rejected"
    if "unredacted secret assignment" in message:
        return "secret-assignment-rejected"
    if "symlink" in message or "unsafe" in message or "forbidden" in message:
        return "source-policy-rejected"
    if "exceeds" in message or "budget" in message or "limit" in message:
        return "source-limit-rejected"
    return "source-validation-rejected"


def _manifest(
    records: list[dict[str, Any]],
    errors: list[dict[str, str]],
    rejected: list[dict[str, str]],
) -> dict[str, Any]:
    return {
        "schema": 1,
        "sources": records,
        "collectionErrors": errors,
        "collectionStatus": "partial" if errors or rejected else "complete",
        "rejectedSources": rejected,
        "omitted": [
            "browser request/response bodies and headers",
            "cookies, storage state, screenshots, traces, HAR files",
            "credentials, secret values, payloads, and unallowlisted database fields",
        ],
        "credentialScan": "pending",
    }


def _write_manifest(output: Path, value: dict[str, Any]) -> None:
    path = output / "manifest.json"
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    path.chmod(0o600)


def _archive(output: Path, archive: Path) -> None:
    if not archive.is_absolute() or archive.exists() or archive.is_symlink():
        raise CollectionError(f"archive destination is unsafe: {archive}")
    try:
        archive.relative_to(output)
    except ValueError:
        pass
    else:
        raise CollectionError("archive destination must be outside the output directory")
    archive.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = archive.with_name(f".{archive.name}.tmp-{os.getpid()}")
    if temporary.exists() or temporary.is_symlink():
        raise CollectionError("archive staging path already exists")
    try:
        with temporary.open("wb") as raw_archive, gzip.GzipFile(
            fileobj=raw_archive, mode="wb", mtime=0
        ) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as target:
                for path in sorted(output.rglob("*")):
                    if path.is_symlink():
                        raise CollectionError(
                            f"retained bundle contains an unsafe member: {path}"
                        )
                    if not path.is_file():
                        continue
                    relative = path.relative_to(output)
                    info = target.gettarinfo(
                        str(path), arcname=PurePath("cooking-diagnostics") / relative
                    )
                    info.mtime = 0
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    with path.open("rb") as input_file:
                        target.addfile(info, input_file)
        temporary.chmod(0o600)
        os.rename(temporary, archive)
    except Exception:
        temporary.unlink(missing_ok=True)
        raise


def collect(
    output_dir: Path,
    sources: list[str],
    snapshot: Path | None,
    snapshot_status: Path | None,
    archive: Path | None,
    missing_sources: list[str] | None = None,
) -> int:
    missing_sources = [] if missing_sources is None else missing_sources
    if (
        not output_dir.is_absolute()
        or _has_symlink_ancestor(output_dir)
        or output_dir.exists()
        or output_dir.is_symlink()
    ):
        raise CollectionError("output directory already exists or is unsafe")
    staging = output_dir.with_name(f".{output_dir.name}.staging-{os.getpid()}")
    if staging.exists() or staging.is_symlink():
        raise CollectionError("staging directory already exists")
    staging.mkdir(mode=0o700, parents=False)
    records: list[dict[str, Any]] = []
    errors: list[dict[str, str]] = []
    rejected: list[dict[str, str]] = []
    seen_labels: set[str] = set()
    total = 0
    try:
        sources_dir = staging / "sources"
        sources_dir.mkdir(mode=0o700)
        for raw in sources:
            candidate_label, separator, candidate_path = raw.partition("=")
            if candidate_label in seen_labels:
                raise CollectionError(f"source label is duplicated: {candidate_label}")
            if not separator or candidate_label not in ALLOWED_LABELS:
                raise CollectionError("source label is not allowlisted")
            if separator and candidate_label in ALLOWED_LABELS:
                seen_labels.add(candidate_label)
            try:
                if separator and candidate_path.startswith("/") and _has_symlink_ancestor(
                    Path(candidate_path)
                ):
                    raise CollectionError("source path has an unsafe symlink ancestor")
                if (
                    separator
                    and candidate_label in ALLOWED_LABELS
                    and candidate_path.startswith("/")
                    and not Path(candidate_path).exists()
                ):
                    errors.append({"label": candidate_label, "status": "missing"})
                    continue
                label, source_path = _parse_source(raw)
                destination = sources_dir / label
                if label == "browser-summary":
                    size, digest = _project_browser_summary(source_path, destination)
                elif label == "runtime-structured":
                    size, digest = _project_runtime_structured(source_path, destination)
                else:
                    size, digest = _project_text(source_path, destination)
            except (CollectionError, OSError, ValueError) as error:
                destination = sources_dir / candidate_label
                destination.unlink(missing_ok=True)
                rejected.append(
                    {
                        "label": candidate_label,
                        "reason": _source_rejection_reason(error),
                        "status": "rejected",
                    }
                )
                continue
            total += size
            if total > MAX_TOTAL_BYTES:
                raise CollectionError("combined evidence exceeds retention limit")
            records.append(
                {"label": label, "path": f"sources/{label}", "bytes": size, "sha256": digest}
            )
        for raw in missing_sources:
            label, separator, raw_path = raw.partition("=")
            if not separator or label not in {"lineage", "lineage-status"}:
                raise CollectionError("missing source must use LINEAGE or LINEAGE-STATUS=ABSOLUTE_PATH")
            if label in seen_labels:
                raise CollectionError(f"source label is duplicated: {label}")
            seen_labels.add(label)
            path = Path(raw_path)
            if not path.is_absolute() or path.is_symlink() or _has_symlink_ancestor(path):
                raise CollectionError("missing source path is unsafe")
            if path.exists():
                raise CollectionError(f"missing source is present: {path}")
            errors.append({"label": label, "status": "missing"})
        if snapshot is not None:
            destination = staging / "lineage.jsonl"
            rows, digest = _canonical_snapshot(snapshot, destination)
            size = destination.stat().st_size
            total += size
            if total > MAX_TOTAL_BYTES:
                raise CollectionError("combined evidence exceeds retention limit")
            records.append({"label": "lineage", "path": "lineage.jsonl", "rows": rows, "bytes": size, "sha256": digest})
        if snapshot_status is not None:
            destination = staging / "lineage-status.json"
            size, digest = _canonical_snapshot_status(snapshot_status, destination)
            total += size
            if total > MAX_TOTAL_BYTES:
                raise CollectionError("combined evidence exceeds retention limit")
            records.append(
                {
                    "label": "lineage-status",
                    "path": "lineage-status.json",
                    "bytes": size,
                    "sha256": digest,
                }
            )
        if not records:
            raise CollectionError("at least one evidence source is required")
        value = _manifest(records, errors, rejected)
        _write_manifest(staging, value)
        EVIDENCE.main([str(staging)])
        value["credentialScan"] = "passed"
        _write_manifest(staging, value)
        if archive is not None:
            _archive(staging, archive)
        os.rename(staging, output_dir)
        return 0
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--source", action="append", default=[], metavar="LABEL=PATH")
    parser.add_argument("--missing-source", action="append", default=[], metavar="LINEAGE=PATH")
    parser.add_argument("--snapshot-jsonl", type=Path)
    parser.add_argument("--snapshot-status", type=Path)
    parser.add_argument("--archive", type=Path)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        return collect(
            args.output_dir,
            args.source,
            args.snapshot_jsonl,
            args.snapshot_status,
            args.archive,
            args.missing_source,
        )
    except (CollectionError, OSError, RuntimeError, ValueError) as error:
        message = str(error)
        for pattern in EVIDENCE.PATTERNS:
            message = message.replace(pattern.decode("ascii"), "[REDACTED]")
        print(f"Cooking diagnostics collection failed: {message}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
