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
EXIT_LIMITS = {"exit_code": 255, "exit_signal": 64}
LABEL_RE = re.compile(r"^[a-z][a-z0-9_-]{0,47}$")
STATUS_RE = re.compile(r"^[a-z][a-z0-9_:-]{0,63}$")
# Rust `time::OffsetDateTime::to_string()` emits single-digit hours without
# padding; keep accepting that canonical producer format for early-UTC runs.
TIMESTAMP_RE = re.compile(
    r"^\d{4}-\d{2}-\d{2}[ T](?:[01]?\d|2[0-3]):[0-5]\d:[0-5]\d"
    r"(?:\.\d{1,9})?(?:Z| Z| [+-]\d{2}:\d{2}(?::\d{2})?)$"
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
        "evidence-scan",
        "gate-results",
        "phase-timing",
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
        "exit_code",
        "exit_signal",
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
    {
        "attempt_completed_at",
        "run_outcome",
        "next_eligible_at",
        "terminal_at",
        "exit_code",
        "exit_signal",
    }
)
EXIT_FIELDS = frozenset({"exit_code", "exit_signal"})
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
RETRY_MARKER_RE = re.compile(r"HEPH_COOKING_RETRY")
RETRY_FIELD_ORDER = (
    "event",
    "classification",
    "lookup_status",
    "event_id",
    "attempt_id",
    "attempt_number",
    "run_id",
    "attempt_state",
    "run_state",
    "run_outcome",
    "exit_code",
    "exit_signal",
)
RETRY_ENUMS = {
    "event": frozenset({"terminal"}),
    "classification": frozenset(
        {
            "retry-terminal-failed",
            "retry-terminal-uncertain",
            "retry-terminal-unresolved",
            "retry-completion-timeout",
        }
    ),
    "lookup_status": frozenset({"ok", "missing", "query-failed", "query-timeout"}),
    "attempt_state": frozenset({"leased", "running", "completed", "failed", "uncertain", "unknown"}),
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
            "unknown",
        }
    ),
    "run_outcome": frozenset({"none", "succeeded", "failed", "cancelled", "unknown"}),
}
RETRY_UUID_FIELDS = frozenset({"event_id", "run_id"})
RETRY_OPTIONAL_UUID_FIELDS = frozenset({"attempt_id"})
RETRY_INTEGER_FIELDS = frozenset({"attempt_number"})
RETRY_EXIT_FIELDS = frozenset({"exit_code", "exit_signal"})
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
        "stage",
        "remaining_seconds",
        "reserve_seconds",
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
RUST_TEST_NAME_RE = r"[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*"
RUST_TEST_RESULT_RE = re.compile(
    rf"^test\s+(?P<test>{RUST_TEST_NAME_RE})\s+\.\.\.\s+(?P<status>ok|FAILED|ignored)$"
)
RUST_PANIC_LOCATION_RE = re.compile(
    r"^thread\s+'[^']{1,128}'\s+panicked at\s+"
    r"(?P<location>[A-Za-z0-9_./:-]+:\d+(?::\d+)?)(?:$|:.*$)"
)
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
SHELL_FAILURE_MARKER = "HEPH_GCP_SHELL_FAILURE"
SHELL_FAILURE_FIELDS = (
    "script", "component", "operation", "reason", "exit_code", "line",
)
SHELL_FAILURE_SCRIPTS = frozenset({"cooking-run", "gateway-libkrun-e2e", "libkrun-integration"})
SHELL_FAILURE_COMPONENTS = frozenset({"cooking", "gateway", "libkrun"})
SHELL_FAILURE_OPERATIONS = frozenset(
    {"preflight", "command", "cgroup-events", "runtime-cleanup", "cgroup-cleanup", "network-integrity", "gateway-cleanup", "cooking-cleanup"}
)
SHELL_FAILURE_REASONS = frozenset(
    {"command-failed", "missing-input", "assertion-mismatch", "network-mismatch", "read-failed", "signal", "process-exit", "timeout"}
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
    {
        "status", "suite", "test", "phase", "duration_ms", "exit_code", "error", "stack",
        "started_at", "finished_at", "component", "result_origin", "report_state",
        "counts", "observed_phases", "passed_phases", "failure_metadata",
    }
)
BROWSER_REPORT_STATES = frozenset(
    {"complete", "missing", "partial", "malformed", "truncated", "report-error"}
)
BROWSER_COUNT_FIELDS = frozenset({"passed", "failed", "skipped", "timed_out"})
BROWSER_PHASE_VALUES = frozenset({"initial", "post-operation"})
BROWSER_FAILURE_FIELDS = frozenset(
    {
        "test_id", "phase", "status", "error_class", "matcher", "source_file",
        "source_line", "source_column", "source_location_kind",
    }
)
BROWSER_TEST_IDS = frozenset({"cooking-live-review", "cooking-post-operation"})
BROWSER_PHASES = frozenset({"initial", "post-operation"})
BROWSER_FAILURE_STATUSES = frozenset({"failed", "timed_out"})
BROWSER_ERROR_CLASSES = frozenset({"assertion", "timeout", "hook", "runtime", "unknown"})
BROWSER_MATCHERS = frozenset(
    {"toBe", "toBeEmpty", "toBeVisible", "toContainText", "toHaveCount", "toHaveURL", "toMatch", "unknown"}
)
BROWSER_SOURCE_LOCATION_KINDS = frozenset({"error", "test"})
BROWSER_SOURCE_RE = re.compile(
    r"^e2e/playwright/cooking-tests/cooking-(?:live-review|post-operation)\.spec\.ts$"
)

EVIDENCE_SCAN_FIELDS = frozenset(
    {"schema", "status", "rule", "file_class", "path_sha256", "checked_files", "checked_bytes"}
)
EVIDENCE_SCAN_STATUSES = frozenset({"passed", "failed", "unavailable", "error"})
EVIDENCE_SCAN_RULES = frozenset(
    {
        "none",
        "browser-secret-org",
        "browser-secret-project",
        "golden-provider-sentinel",
        "cooking-inbound-sentinel",
        "cooking-model-sentinel",
        "cooking-relay-sentinel",
        "cooking-model-rotated-sentinel",
        "cooking-inbound-rotated-sentinel",
        "cooking-relay-rotated-sentinel",
        "fixture-credential",
        "archive-nesting-limit",
        "archive-size-limit",
        "archive-member-size-limit",
        "archive-invalid",
        "evidence-root",
        "symlink",
        "file-size-limit",
        "no-files",
        "read-error",
        "scan-error",
        "scanner-unavailable",
        "scanner-error",
        "missing-result",
        "status-report-unavailable",
    }
)
EVIDENCE_SCAN_FILE_CLASSES = frozenset(
    {"none", "archive", "content", "structured", "text", "binary", "directory", "filesystem", "metadata", "unknown"}
)
EVIDENCE_SCAN_DIGEST_RE = re.compile(r"^[0-9a-f]{64}$")
EVIDENCE_SCAN_MAX_FILES = 1_000_000
EVIDENCE_SCAN_MAX_BYTES = 2**63 - 1

GATE_RESULTS_FIELDS = frozenset(
    {
        "schema",
        "revision",
        "script_sha256",
        "test_mode",
        "overall_exit_code",
        "supervisor_exit_code",
        "finalized",
        "gates",
    }
)
GATE_RESULT_FIELDS = frozenset({"state", "exit_code", "reason_class"})
GATE_NAMES = ("workload", "evidence-scan", "browser-validation")
GATE_STATES = frozenset({"passed", "failed", "timed-out", "unknown"})
GATE_MODES = frozenset({"diagnostic", "gcp-cooking"})
GATE_REVISION_RE = re.compile(r"^[0-9a-f]{40}$")
GATE_SCRIPT_SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
GATE_REASON_VALUES = frozenset(
    {
        "none",
        "unknown",
        "unfinished",
        "workload-failed",
        "evidence-scan-failed",
        "evidence-scan-report-invalid",
        "browser-validation-failed",
        "browser-report-invalid",
        "browser-tests-not-passed",
        "timeout",
        "supervisor-failed",
    }
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


COLLECTOR_FAILURE_REASONS = frozenset(
    {
        "all-sources-rejected",
        "retention-limit",
        "output-path-invalid",
        "snapshot-validation",
        "archive-create",
        "final-scan",
        "internal-error",
    }
)


def _collector_failure_marker(error: Exception) -> str:
    """Return only a fixed, non-sensitive classification for a fatal error."""

    scan_failure = getattr(EVIDENCE, "ScanFailure", None)
    if scan_failure is not None and isinstance(error, scan_failure):
        stage, reason = "final-scan", "final-scan"
    else:
        # Exception text is used only for local classification.  It is never
        # emitted: paths, payloads, and provider/secret details stay out of
        # the serial log even when a fatal path is reached.
        message = str(error).lower()
        if "at least one evidence source" in message:
            stage, reason = "collection", "all-sources-rejected"
        elif "exceeds retention" in message or "retention limit" in message:
            stage, reason = "retention", "retention-limit"
        elif "snapshot" in message or "lineage" in message:
            stage, reason = "snapshot", "snapshot-validation"
        elif "archive" in message:
            stage, reason = "archive", "archive-create"
        elif "output directory" in message or "staging directory" in message:
            stage, reason = "validation", "output-path-invalid"
        else:
            stage, reason = "collection", "internal-error"
    assert reason in COLLECTOR_FAILURE_REASONS
    return (
        "HEPH_GCP_DIAGNOSTICS event=collector-failure operation=collection "
        f"stage={stage} reason_class={reason} status=failed exit_code=1"
    )


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
    if field in EXIT_FIELDS:
        if type(value) is not int or not 0 <= value <= EXIT_LIMITS[field]:
            raise CollectionError(f"snapshot {field} is invalid")
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


def _project_retry_marker(line: str) -> str:
    """Project the scenario's terminal retry marker into typed fields only."""

    match = RETRY_MARKER_RE.search(line)
    if match is None:
        raise CollectionError("retry marker is missing")
    prefix = line[: match.start()].rstrip()
    if prefix and (prefix[-1].isalnum() or prefix[-1] in "_=-\"'([{"):
        raise CollectionError("retry marker is quoted or embedded in a payload")
    tokens = line[match.start() :].split()
    if not tokens or tokens[0] != "HEPH_COOKING_RETRY":
        raise CollectionError("retry marker is malformed")
    fields: dict[str, str] = {}
    for token in tokens[1:]:
        key, separator, value = token.partition("=")
        if not separator or not key or not value or key in fields:
            raise CollectionError("retry marker fields are malformed")
        fields[key] = value
    if set(fields) != set(RETRY_FIELD_ORDER):
        raise CollectionError("retry marker fields are not allowlisted")

    safe: dict[str, str] = {}
    for field in RETRY_FIELD_ORDER:
        value = fields[field]
        if field in RETRY_ENUMS:
            if value not in RETRY_ENUMS[field]:
                raise CollectionError(f"retry marker enum is invalid: {field}")
            safe[field] = value
        elif field in RETRY_UUID_FIELDS:
            if not UUID_RE.fullmatch(value):
                raise CollectionError(f"retry marker identifier is invalid: {field}")
            safe[field] = value.lower()
        elif field in RETRY_OPTIONAL_UUID_FIELDS:
            if value == "unknown":
                safe[field] = value
            elif UUID_RE.fullmatch(value):
                safe[field] = value.lower()
            else:
                raise CollectionError(f"retry marker identifier is invalid: {field}")
        elif field in RETRY_INTEGER_FIELDS:
            if value == "unknown":
                safe[field] = value
            elif value.isascii() and value.isdecimal() and 1 <= int(value) <= 100:
                safe[field] = str(int(value))
            else:
                raise CollectionError(f"retry marker integer is invalid: {field}")
        elif field in RETRY_EXIT_FIELDS:
            if value in {"none", "unknown"}:
                safe[field] = value
            elif (
                value.isascii()
                and value.isdecimal()
                and 0 <= int(value) <= EXIT_LIMITS[field]
            ):
                safe[field] = str(int(value))
            else:
                raise CollectionError(f"retry marker exit value is invalid: {field}")
        else:  # pragma: no cover - RETRY_FIELD_ORDER is closed above.
            raise CollectionError(f"retry marker field is not implemented: {field}")
    return "HEPH_COOKING_RETRY " + " ".join(f"{field}={safe[field]}" for field in RETRY_FIELD_ORDER)


def _project_evidence_scan_marker(line: str) -> str:
    """Project the Cooking runner's typed scanner marker without raw fields."""

    tokens = line.split()
    if not tokens or tokens[0] != "HEPH_GCP_COOKING":
        raise CollectionError("evidence scan marker is malformed")
    fields: dict[str, str] = {}
    for token in tokens[1:]:
        key, separator, value = token.partition("=")
        if not separator or key in fields:
            raise CollectionError("evidence scan marker fields are malformed")
        fields[key] = value
    expected = {
        "event", "operation", "phase", "status", "exit_code", "report_status", "rule",
        "file_class", "path_sha256", "checked_files", "checked_bytes",
    }
    if set(fields) != expected or fields["event"] != "evidence-scan" or fields["operation"] != "evidence-scan":
        raise CollectionError("evidence scan marker fields are not allowlisted")
    if fields["phase"] != "evidence" or fields["status"] not in {"passed", "failed"}:
        raise CollectionError("evidence scan marker outcome is invalid")
    if fields["report_status"] not in {"passed", "failed", "unavailable", "error"}:
        raise CollectionError("evidence scan marker report status is invalid")
    if fields["rule"] not in EVIDENCE_SCAN_RULES:
        raise CollectionError("evidence scan marker rule is invalid")
    if fields["file_class"] not in EVIDENCE_SCAN_FILE_CLASSES:
        raise CollectionError("evidence scan marker file class is invalid")
    if fields["path_sha256"] != "none" and EVIDENCE_SCAN_DIGEST_RE.fullmatch(fields["path_sha256"]) is None:
        raise CollectionError("evidence scan marker path digest is invalid")
    for field, maximum in (("exit_code", 255), ("checked_files", EVIDENCE_SCAN_MAX_FILES), ("checked_bytes", EVIDENCE_SCAN_MAX_BYTES)):
        value = fields[field]
        if not value.isascii() or not value.isdecimal() or int(value) > maximum:
            raise CollectionError(f"evidence scan marker {field} is invalid")
    report_status = fields["report_status"]
    rule = fields["rule"]
    if report_status == "passed" and (rule != "none" or fields["file_class"] != "none" or fields["path_sha256"] != "none"):
        raise CollectionError("passed evidence scan marker has failure fields")
    if report_status != "passed" and rule == "none":
        raise CollectionError("failed evidence scan marker has no failure rule")
    if report_status in {"unavailable", "error"} and rule not in {"scanner-unavailable", "scanner-error", "missing-result", "status-report-unavailable"}:
        raise CollectionError("unavailable evidence scan marker has an invalid rule")
    return (
        "HEPH_GCP_COOKING event=evidence-scan operation=evidence-scan phase=evidence "
        f"status={fields['status']} exit_code={int(fields['exit_code'])} "
        f"report_status={report_status} rule={rule} file_class={fields['file_class']} "
        f"path_sha256={fields['path_sha256']} checked_files={int(fields['checked_files'])} "
        f"checked_bytes={int(fields['checked_bytes'])}"
    )


def _project_browser_validation_marker(line: str) -> str:
    """Project the browser gate marker's fixed result vocabulary."""

    tokens = line.split()
    if not tokens or tokens[0] != "HEPH_GCP_COOKING":
        raise CollectionError("browser validation marker is malformed")
    fields: dict[str, str] = {}
    for token in tokens[1:]:
        key, separator, value = token.partition("=")
        if not separator or key in fields:
            raise CollectionError("browser validation marker fields are malformed")
        fields[key] = value
    expected = {"event", "operation", "phase", "status", "report_state", "reason", "exit_code"}
    if set(fields) != expected or fields["event"] != "browser-report-validation" or fields["operation"] != "browser-report-validation":
        raise CollectionError("browser validation marker fields are not allowlisted")
    if fields["phase"] != "evidence" or fields["status"] not in {"passed", "failed"}:
        raise CollectionError("browser validation marker outcome is invalid")
    if fields["report_state"] not in {"complete", "missing", "partial", "malformed", "truncated", "report-error", "unknown"}:
        raise CollectionError("browser validation marker report state is invalid")
    if fields["reason"] not in {"complete", "invalid-report", "incomplete-phases", "browser-tests-not-passed", "timeout", "report-validation-failed"}:
        raise CollectionError("browser validation marker reason is invalid")
    if not fields["exit_code"].isascii() or not fields["exit_code"].isdecimal() or int(fields["exit_code"]) > 255:
        raise CollectionError("browser validation marker exit code is invalid")
    return (
        "HEPH_GCP_COOKING event=browser-report-validation operation=browser-report-validation "
        f"phase=evidence status={fields['status']} report_state={fields['report_state']} "
        f"reason={fields['reason']} exit_code={int(fields['exit_code'])}"
    )


def _project_workload_result_marker(line: str) -> str:
    """Project the workload gate marker independently of log prefixes."""

    tokens = line.split()
    if not tokens or tokens[0] != "HEPH_GCP_COOKING":
        raise CollectionError("workload result marker is malformed")
    fields: dict[str, str] = {}
    for token in tokens[1:]:
        key, separator, value = token.partition("=")
        if not separator or key in fields:
            raise CollectionError("workload result marker fields are malformed")
        fields[key] = value
    expected = {"event", "operation", "phase", "status", "exit_code"}
    if set(fields) != expected or fields["event"] != "workload-result" or fields["operation"] != "cooking-workload":
        raise CollectionError("workload result marker fields are not allowlisted")
    if fields["phase"] != "cooking" or fields["status"] not in {"passed", "failed"}:
        raise CollectionError("workload result marker outcome is invalid")
    if not fields["exit_code"].isascii() or not fields["exit_code"].isdecimal() or int(fields["exit_code"]) > 255:
        raise CollectionError("workload result marker exit code is invalid")
    return (
        "HEPH_GCP_COOKING event=workload-result operation=cooking-workload phase=cooking "
        f"status={fields['status']} exit_code={int(fields['exit_code'])}"
    )


def _project_gate_marker(line: str) -> str | None:
    """Normalize a known gate marker even when a formatter prefixes it."""

    marker_start = line.find("HEPH_GCP_COOKING ")
    if marker_start < 0:
        return None
    suffix = line[marker_start:]
    if "event=workload-result" in suffix:
        return _project_workload_result_marker(suffix)
    if "event=evidence-scan" in suffix and "report_status=" in suffix:
        return _project_evidence_scan_marker(suffix)
    if "event=browser-report-validation" in suffix:
        return _project_browser_validation_marker(suffix)
    return None


def _project_shell_failure_marker(line: str) -> str | None:
    """Project the shell runner's fixed, argument-free failure marker."""

    marker_start = line.find(SHELL_FAILURE_MARKER + " ")
    if marker_start < 0:
        return None
    tokens = line[marker_start:].split()
    fields: dict[str, str] = {}
    if not tokens or tokens[0] != SHELL_FAILURE_MARKER:
        raise CollectionError("shell failure marker is malformed")
    for token in tokens[1:]:
        key, separator, value = token.partition("=")
        if not separator or key in fields:
            raise CollectionError("shell failure marker fields are malformed")
        fields[key] = value
    if tuple(fields) != SHELL_FAILURE_FIELDS:
        raise CollectionError("shell failure marker fields are not allowlisted")
    if (fields["script"], fields["component"]) not in {
        ("cooking-run", "cooking"),
        ("gateway-libkrun-e2e", "gateway"),
        ("libkrun-integration", "libkrun"),
    }:
        raise CollectionError("shell failure marker source is invalid")
    if fields["operation"] not in SHELL_FAILURE_OPERATIONS or fields["reason"] not in SHELL_FAILURE_REASONS:
        raise CollectionError("shell failure marker classification is invalid")
    if not fields["exit_code"].isascii() or not fields["exit_code"].isdecimal() or not 1 <= int(fields["exit_code"]) <= 255:
        raise CollectionError("shell failure marker exit code is invalid")
    if not fields["line"].isascii() or not fields["line"].isdecimal() or not 1 <= int(fields["line"]) <= 1_000_000:
        raise CollectionError("shell failure marker source line is invalid")
    return (
        f"{SHELL_FAILURE_MARKER} script={fields['script']} component={fields['component']} "
        f"operation={fields['operation']} reason={fields['reason']} "
        f"exit_code={int(fields['exit_code'])} line={int(fields['line'])}"
    )


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
            elif (shell_failure := _project_shell_failure_marker(line)) is not None:
                projected = shell_failure
            elif RETRY_MARKER_RE.search(line) is not None:
                projected = _project_retry_marker(line)
            elif (gate_marker := _project_gate_marker(line)) is not None:
                projected = gate_marker
            elif DROP_LINE_RE.search(line):
                continue
            else:
                marker = RUNTIME_MARKER_RE.search(line)
                stripped = line.strip()
                stack_match = SAFE_STACK_RE.fullmatch(stripped)
                assertion_match = ASSERTION_LINE_RE.fullmatch(stripped)
                error_match = ERROR_LINE_RE.fullmatch(stripped)
                rust_test = RUST_TEST_RESULT_RE.fullmatch(stripped)
                panic_location = RUST_PANIC_LOCATION_RE.fullmatch(stripped)
                if rust_test:
                    status = {"ok": "passed", "FAILED": "failed", "ignored": "ignored"}[rust_test.group("status")]
                    projected = f"HEPH_GCP_TEST test={rust_test.group('test')} status={status}"
                elif panic_location:
                    # A caught panic is only informational until cargo emits a
                    # canonical test result; retain its bounded source location.
                    projected = f"location={panic_location.group('location')}"
                elif stack_match:
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
    if (
        ("report_state" in value or "failure_metadata" in value or "counts" in value)
        and ("error" in value or "stack" in value)
    ):
        raise CollectionError("typed browser summary cannot retain error or stack text")
    safe: dict[str, Any] = {}
    for field, item in value.items():
        if item is None and field not in {"error", "stack"}:
            raise CollectionError(f"browser summary field is invalid: {field}")
        if field == "status":
            if not isinstance(item, str) or item not in {"passed", "failed", "timed_out", "not-run", "unknown"}:
                raise CollectionError(f"browser summary status is invalid: {field}")
        elif field == "phase":
            if not isinstance(item, str) or item not in {"browser", "cooking"}:
                raise CollectionError(f"browser summary phase is invalid: {field}")
        elif field == "component":
            if item != "browser-e2e":
                raise CollectionError(f"browser summary component is invalid: {field}")
        elif field == "result_origin":
            if item not in {"playwright-report", "no-browser-report"}:
                raise CollectionError(f"browser summary result origin is invalid: {field}")
        elif field == "report_state":
            if item not in BROWSER_REPORT_STATES:
                raise CollectionError(f"browser summary report state is invalid: {field}")
        elif field == "counts":
            if not isinstance(item, dict) or set(item) != BROWSER_COUNT_FIELDS:
                raise CollectionError(f"browser summary counts are invalid: {field}")
            if any(type(number) is not int or number < 0 or number > 256 for number in item.values()):
                raise CollectionError(f"browser summary count is invalid: {field}")
            safe[field] = dict(item)
            continue
        elif field == "observed_phases":
            if (
                not isinstance(item, list)
                or len(item) > 2
                or item != sorted(item)
                or any(phase not in BROWSER_PHASE_VALUES for phase in item)
                or len(set(item)) != len(item)
            ):
                raise CollectionError(f"browser summary observed phases are invalid: {field}")
            safe[field] = list(item)
            continue
        elif field == "passed_phases":
            if (
                not isinstance(item, list)
                or len(item) > 2
                or item != sorted(item)
                or any(phase not in BROWSER_PHASE_VALUES for phase in item)
                or len(set(item)) != len(item)
            ):
                raise CollectionError(f"browser summary passed phases are invalid: {field}")
            safe[field] = list(item)
            continue
        elif field == "failure_metadata":
            if not isinstance(item, list) or len(item) > 8:
                raise CollectionError(f"browser summary failure metadata is invalid: {field}")
            projected_failures = []
            for failure in item:
                if not isinstance(failure, dict) or set(failure) != BROWSER_FAILURE_FIELDS:
                    raise CollectionError(f"browser summary failure metadata entry is invalid: {field}")
                if (
                    failure["test_id"] not in BROWSER_TEST_IDS
                    or failure["phase"] not in BROWSER_PHASES
                    or failure["status"] not in BROWSER_FAILURE_STATUSES
                    or failure["error_class"] not in BROWSER_ERROR_CLASSES
                    or failure["matcher"] not in BROWSER_MATCHERS
                    or failure["source_location_kind"] not in BROWSER_SOURCE_LOCATION_KINDS
                    or not isinstance(failure["source_file"], str)
                    or BROWSER_SOURCE_RE.fullmatch(failure["source_file"]) is None
                    or type(failure["source_line"]) is not int
                    or not 1 <= failure["source_line"] <= 100_000
                    or type(failure["source_column"]) is not int
                    or not 1 <= failure["source_column"] <= 10_000
                ):
                    raise CollectionError(f"browser summary failure metadata values are invalid: {field}")
                projected_failures.append(dict(failure))
            safe[field] = projected_failures
            continue
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


def _project_evidence_scan(source: Path, destination: Path) -> tuple[int, str]:
    """Retain the scanner's typed result without paths or scan content."""

    source = _safe_input(source)
    try:
        with _open_safe(source) as input_file:
            raw = input_file.read(MAX_LINE_BYTES + 1)
        if len(raw) > MAX_LINE_BYTES:
            raise CollectionError("evidence scan result exceeds its retention limit")
        EVIDENCE.check_bytes(raw, str(source))
        _reject_secret_assignments(raw)
        value = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise CollectionError("evidence scan result must be one JSON object") from error
    if not isinstance(value, dict) or set(value) != EVIDENCE_SCAN_FIELDS:
        raise CollectionError("evidence scan result contains an unallowlisted field")
    if value.get("schema") != 1:
        raise CollectionError("evidence scan result schema is invalid")
    status = value.get("status")
    if status not in EVIDENCE_SCAN_STATUSES:
        raise CollectionError("evidence scan result status is invalid")
    rule = value.get("rule")
    if rule not in EVIDENCE_SCAN_RULES:
        raise CollectionError("evidence scan result rule is invalid")
    file_class = value.get("file_class")
    if file_class not in EVIDENCE_SCAN_FILE_CLASSES:
        raise CollectionError("evidence scan result file class is invalid")
    path_digest = value.get("path_sha256")
    if path_digest is not None and (
        not isinstance(path_digest, str) or EVIDENCE_SCAN_DIGEST_RE.fullmatch(path_digest) is None
    ):
        raise CollectionError("evidence scan result path digest is invalid")
    for field in ("checked_files", "checked_bytes"):
        count = value.get(field)
        if type(count) is not int or not 0 <= count <= (EVIDENCE_SCAN_MAX_FILES if field == "checked_files" else EVIDENCE_SCAN_MAX_BYTES):
            raise CollectionError(f"evidence scan result {field} is invalid")
    if status == "passed" and (rule != "none" or file_class != "none" or path_digest is not None):
        raise CollectionError("passed evidence scan result has failure fields")
    if status != "passed" and rule == "none":
        raise CollectionError("failed evidence scan result has no failure rule")
    if status in {"unavailable", "error"} and rule not in {"scanner-unavailable", "scanner-error", "missing-result", "status-report-unavailable"}:
        raise CollectionError("unavailable evidence scan result has an invalid rule")
    safe = {
        "schema": 1,
        "status": status,
        "rule": rule,
        "file_class": file_class,
        "path_sha256": path_digest,
        "checked_files": value["checked_files"],
        "checked_bytes": value["checked_bytes"],
    }
    encoded = (json.dumps(safe, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
    destination.write_bytes(encoded)
    destination.chmod(0o600)
    return len(encoded), hashlib.sha256(encoded).hexdigest()


def _validate_gate_result(name: str, value: Any) -> dict[str, Any]:
    if name not in GATE_NAMES or not isinstance(value, dict) or set(value) != GATE_RESULT_FIELDS:
        raise CollectionError("gate result fields are invalid")
    state = value.get("state")
    exit_code = value.get("exit_code")
    reason = value.get("reason_class")
    if (
        not isinstance(state, str)
        or state not in GATE_STATES
        or not isinstance(reason, str)
        or reason not in GATE_REASON_VALUES
    ):
        raise CollectionError("gate result classification is invalid")
    if exit_code is not None and (type(exit_code) is not int or not 0 <= exit_code <= 255):
        raise CollectionError("gate result exit code is invalid")
    if state == "passed" and (exit_code != 0 or reason != "none"):
        raise CollectionError("passed gate result is inconsistent")
    if state == "failed" and (
        type(exit_code) is not int
        or exit_code == 0
        or reason
        not in {
            "workload-failed",
            "evidence-scan-failed",
            "evidence-scan-report-invalid",
            "browser-validation-failed",
            "browser-report-invalid",
            "browser-tests-not-passed",
            "supervisor-failed",
        }
    ):
        raise CollectionError("failed gate result is inconsistent")
    if state == "timed-out" and (exit_code != 124 or reason != "timeout"):
        raise CollectionError("timed-out gate result is inconsistent")
    if state == "unknown" and (exit_code is not None or reason not in {"unknown", "unfinished"}):
        raise CollectionError("unknown gate result is inconsistent")
    return {"state": state, "exit_code": exit_code, "reason_class": reason}


def _validate_gate_results(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != GATE_RESULTS_FIELDS:
        raise CollectionError("gate results contain an unallowlisted field")
    if type(value.get("schema")) is not int or value["schema"] != 1:
        raise CollectionError("gate results schema is invalid")
    revision = value.get("revision")
    script_sha256 = value.get("script_sha256")
    if not isinstance(revision, str) or GATE_REVISION_RE.fullmatch(revision) is None:
        raise CollectionError("gate results revision is invalid")
    if not isinstance(script_sha256, str) or GATE_SCRIPT_SHA256_RE.fullmatch(script_sha256) is None:
        raise CollectionError("gate results script digest is invalid")
    test_mode = value.get("test_mode")
    if not isinstance(test_mode, str) or test_mode not in GATE_MODES:
        raise CollectionError("gate results test mode is invalid")
    overall_exit = value.get("overall_exit_code")
    supervisor_exit = value.get("supervisor_exit_code")
    for field, exit_code in (("overall_exit_code", overall_exit), ("supervisor_exit_code", supervisor_exit)):
        if type(exit_code) is not int or not 0 <= exit_code <= 255:
            raise CollectionError(f"gate results {field} is invalid")
    if value.get("finalized") is not True:
        raise CollectionError("gate results are not finalized")
    gates = value.get("gates")
    if not isinstance(gates, dict) or set(gates) != set(GATE_NAMES):
        raise CollectionError("gate results must contain exactly three gates")
    safe_gates = {name: _validate_gate_result(name, gates[name]) for name in GATE_NAMES}
    states = [safe_gates[name]["state"] for name in GATE_NAMES]
    if overall_exit == 0 and states != ["passed", "passed", "passed"]:
        raise CollectionError("zero exit requires three passed gates")
    if overall_exit != 0 and all(state == "passed" for state in states):
        raise CollectionError("nonzero exit requires a non-passed gate")
    return {
        "schema": 1,
        "revision": revision,
        "script_sha256": script_sha256,
        "test_mode": test_mode,
        "overall_exit_code": overall_exit,
        "supervisor_exit_code": supervisor_exit,
        "finalized": True,
        "gates": safe_gates,
    }


def _project_gate_results(source: Path, destination: Path) -> tuple[int, str]:
    """Retain the finalized supervisor gate contract without raw diagnostics."""

    source = _safe_input(source)
    try:
        with _open_safe(source) as input_file:
            raw = input_file.read(MAX_LINE_BYTES + 1)
        if len(raw) > MAX_LINE_BYTES:
            raise CollectionError("gate results exceed their retention limit")
        EVIDENCE.check_bytes(raw, str(source))
        _reject_secret_assignments(raw)
        value = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise CollectionError("gate results must be one JSON object") from error
    safe = _validate_gate_results(value)
    encoded = (json.dumps(safe, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
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


def _project_phase_timing(source: Path, destination: Path) -> tuple[int, str]:
    """Validate and retain only the safe phase-timing projection."""

    source = _safe_input(source)
    helper_path = Path(__file__).with_name("gcp_phase_timing.py")
    if helper_path.is_symlink() or not helper_path.is_file():
        raise CollectionError("phase timing validator is unavailable")
    spec = importlib.util.spec_from_file_location("gcp_phase_timing", helper_path)
    if spec is None or spec.loader is None:
        raise CollectionError("phase timing validator cannot be loaded")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    try:
        safe = module.validate_projection(json.loads(source.read_text(encoding="utf-8")))
    except (OSError, ValueError, module.TimingError) as error:
        raise CollectionError("phase timing records are invalid") from error
    encoded = (json.dumps(safe, sort_keys=True, separators=(",", ":")) + "\n").encode()
    if len(encoded) > 64 * 1024:
        raise CollectionError("phase timing projection exceeds its retention limit")
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


def _snapshot_rejection_reason(error: Exception) -> str:
    """Map snapshot validation failures to closed, non-sensitive classes."""

    if isinstance(error, OSError):
        return "snapshot-read"
    message = str(error).lower()
    # These are the only classes whose messages include caller-controlled
    # paths. Check their controlled prefixes before field-name classes.
    if message.startswith("source cannot be opened safely"):
        return "snapshot-read"
    if message.startswith("source must be") or message.startswith("source metadata"):
        return "snapshot-path"
    if "invalid json" in message or "one json object" in message:
        return "snapshot-invalid-json"
    if (
        "row budget" in message
        or "retention limit" in message
        or "source exceeds" in message
        or "row count" in message
    ):
        return "snapshot-row-limit"
    if (
        "unallowlisted field" in message
        or "snapshot field cannot be" in message
        or "timestamp" in message
    ):
        return "snapshot-schema"
    if "attempt number" in message:
        return "snapshot-enum"
    if "identifier" in message:
        return "snapshot-identifier"
    if "status" in message and "classification" in message:
        return "snapshot-status"
    if "status" in message and ("recognized" in message or "invalid" in message):
        return "snapshot-enum"
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
                elif label == "evidence-scan":
                    size, digest = _project_evidence_scan(source_path, destination)
                elif label == "gate-results":
                    size, digest = _project_gate_results(source_path, destination)
                elif label == "runtime-structured":
                    size, digest = _project_runtime_structured(source_path, destination)
                elif label == "phase-timing":
                    size, digest = _project_phase_timing(source_path, destination)
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
            try:
                rows, digest = _canonical_snapshot(snapshot, destination)
                size = destination.stat().st_size
            except (CollectionError, OSError, ValueError) as error:
                # A malformed producer snapshot is one source failure. Keep
                # independent serial/runtime evidence and report the typed
                # rejection; never retain a partially projected JSONL file.
                destination.unlink(missing_ok=True)
                rejected.append(
                    {
                        "label": "lineage",
                        "reason": _snapshot_rejection_reason(error),
                        "status": "rejected",
                    }
                )
            else:
                total += size
                if total > MAX_TOTAL_BYTES:
                    raise CollectionError("combined evidence exceeds retention limit")
                records.append({"label": "lineage", "path": "lineage.jsonl", "rows": rows, "bytes": size, "sha256": digest})
        if snapshot_status is not None:
            destination = staging / "lineage-status.json"
            try:
                size, digest = _canonical_snapshot_status(snapshot_status, destination)
            except (CollectionError, OSError, ValueError) as error:
                # See the lineage JSONL handling above. Status is retained
                # only after its complete object passes the same strict
                # schema validation.
                destination.unlink(missing_ok=True)
                rejected.append(
                    {
                        "label": "lineage-status",
                        "reason": _snapshot_rejection_reason(error),
                        "status": "rejected",
                    }
                )
            else:
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
        print(_collector_failure_marker(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
