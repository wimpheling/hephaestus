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
    "browser-summary", "test-output", "evidence-scan", "gate-results", "lineage", "lineage-status",
}
SAFE_STATUS = COLLECTOR.SNAPSHOT_STATUS_VALUES
TRIAGE_FIELDS = {
    "schema", "collectionStatus", "rejectedSources", "denial", "attempts", "snapshotStatus", "retry", "sources", "failures",
    "browserObservations", "browser", "evidenceScan", "gateResults", "runtimeResults",
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
    "disposition", "next_eligible_at", "terminal_at", "sampled_at", "exit_code", "exit_signal",
}
FAILURE_SOURCE_LABELS = {
    "serial", "host-journal", "runtime-log", "runtime-structured",
    "browser-summary", "test-output",
}
FAILURE_MARKERS = {
    "HEPH_GCP_TEST",
    "HEPH_GCP_RUNTIME",
    "HEPH_GCP_KVM_BUILD_ERROR",
    "HEPH_GCP_KVM_FIRST_ERROR",
    "HEPH_GCP_KVM_SMOKE",
    "HEPH_GCP_RUNNER_IMAGE_READINESS",
    "HEPH_GCP_DIAGNOSTICS",
    "HEPH_GCP_COOKING",
    "HEPHAESTUS_GCP_COOKING",
}
FAILURE_FIELDS = {
    "test", "test_result", "status", "phase", "error_class", "location", "run_id",
    "attempt_run_id", "exit", "exit_code", "exit_signal", "event", "operation", "reason_class", "class", "tool",
    "component", "result_origin", "duration_ms", "stage", "remaining_seconds", "reserve_seconds",
}
FAILURE_STATUS_VALUES = {"failed", "error", "timeout", "timed-out", "timed_out", "nonzero"}
FAILURE_COMPONENT_VALUES = {"browser-e2e"}
FAILURE_ORIGIN_VALUES = {"playwright-report", "no-browser-report"}
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

# Browser output is retained by the collector only after its own safe-line
# projection.  Triage applies a second, positive projection here: only these
# fixed matcher names, timeout class, and repository test locations can leave
# the private bundle.  In particular, an assertion/location is never joined to
# a failure from another source (or even inferred to be its cause).
BROWSER_OBSERVATION_SOURCES = {"runtime-log", "test-output"}
BROWSER_OBSERVATION_LIMIT = 64
BROWSER_MATCHERS = frozenset({
    "toBe", "toEqual", "toStrictEqual", "toContain", "toHaveText", "toHaveURL",
    "toHaveValue", "toBeVisible", "toBeHidden", "toBeTruthy", "toBeFalsy",
    "toHaveLength", "toBeDefined", "toBeNull", "toBeUndefined", "toMatch",
    "toHaveAttribute", "toBeChecked", "toBeDisabled", "toBeEnabled", "toBeEditable",
    "toBeEmpty", "toBeFocused", "toBeInViewport", "toBeAttached", "toHaveClass",
    "toHaveCount", "toHaveId", "toHaveRole", "toPass",
})
_BROWSER_EXPR = r"(?:received|expected|locator|page|[A-Za-z_][A-Za-z0-9_.-]{0,63})"
BROWSER_ASSERTION_RE = re.compile(
    rf"^expect\({_BROWSER_EXPR}\)\.(?P<matcher>[A-Za-z][A-Za-z0-9_]{{1,31}})"
    rf"\((?:{_BROWSER_EXPR})?\)(?: failed)?$"
)
BROWSER_TIMEOUT_RE = re.compile(
    r"^(?:Test )?timeout(?: of [0-9]{1,7}ms| [0-9]{1,7}ms) exceeded\.?$",
    re.IGNORECASE,
)
BROWSER_WAIT_TIMEOUT_RE = re.compile(
    rf"^Timed out [0-9]{{1,7}}ms waiting for expect\({_BROWSER_EXPR}\)\."
    rf"(?P<matcher>[A-Za-z][A-Za-z0-9_]{{1,31}})\(\)$"
)
# The browser job runs from e2e/playwright.  Accept its relative paths and
# the equivalent repository-prefixed or absolute form, then retain only the
# basename.  The absolute prefix is deliberately component-bounded and must
# end at the repository's e2e/playwright directory.
BROWSER_LOCATION_RE = re.compile(
    r"^(?:(?:/[A-Za-z0-9_.-]+)+/e2e/playwright/(?:tests|cooking-tests)/|"
    r"(?:e2e/playwright/)?(?:tests|cooking-tests)/|)"
    r"(?P<file>[A-Za-z0-9_-][A-Za-z0-9_.-]{0,127}\.spec\.ts):"
    r"(?P<line>[1-9][0-9]{0,5}):(?P<column>[1-9][0-9]{0,5})$"
)


def _browser_observation(source: str, order: int, line: str) -> dict[str, Any] | None:
    """Return one bounded typed browser observation, never its source text."""

    if len(line) > 1024:
        return None
    kind, _, value = line.partition("=")
    if kind in {"error", "assertion"}:
        assertion = BROWSER_ASSERTION_RE.fullmatch(value)
        if assertion is not None:
            matcher = assertion.group("matcher")
            if matcher in BROWSER_MATCHERS:
                return {
                    "source": source,
                    "order": order,
                    "kind": "assertion",
                    "error_class": "assertion-failure",
                    "matcher": matcher,
                }
        if kind == "error" and BROWSER_TIMEOUT_RE.fullmatch(value) is not None:
            return {
                "source": source,
                "order": order,
                "kind": "error",
                "error_class": "timeout",
            }
        if kind == "error":
            timed_out = BROWSER_WAIT_TIMEOUT_RE.fullmatch(value)
            if timed_out is not None and timed_out.group("matcher") in BROWSER_MATCHERS:
                return {
                    "source": source,
                    "order": order,
                    "kind": "error",
                    "error_class": "timeout",
                    "matcher": timed_out.group("matcher"),
                }
        return None
    if kind == "location":
        location = BROWSER_LOCATION_RE.fullmatch(value)
        if location is None:
            return None
        return {
            "source": source,
            "order": order,
            "kind": "location",
            "file": location.group("file"),
            "line": int(location.group("line")),
            "column": int(location.group("column")),
        }
    return None


def _project_browser_observations(
    root: Path,
    source_records: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Project safe browser observations independently for each source.

    ``order`` is the 1-based line order within the named source.  Keeping the
    source and order makes observations auditable without correlating unrelated
    records or publishing any browser error/assertion text.
    """

    observations: list[dict[str, Any]] = []
    for record in source_records:
        source = record.get("label")
        if source not in BROWSER_OBSERVATION_SOURCES:
            continue
        path = _safe_path(root, record["path"])
        for order, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            projected = _browser_observation(source, order, line.strip())
            if projected is not None:
                observations.append(projected)
                if len(observations) >= BROWSER_OBSERVATION_LIMIT:
                    return observations
    return observations
BROWSER_SUMMARY_STATUS = {"passed", "failed", "timed_out", "not-run", "unknown"}
BROWSER_SUMMARY_PHASES = {"browser", "cooking"}
BROWSER_SUMMARY_ORIGINS = {"playwright-report", "no-browser-report"}
BROWSER_SUMMARY_STATES = {
    "complete", "missing", "partial", "malformed", "truncated", "report-error", "unknown",
}
BROWSER_SUMMARY_COUNT_FIELDS = {"passed", "failed", "skipped", "timed_out"}
BROWSER_SUMMARY_PHASE_VALUES = {"initial", "post-operation"}
BROWSER_SUMMARY_REASON_CLASSES = {
    "browser-tests-not-passed", "browser-tests-failed", "clean-exit", "complete",
    "incomplete-phases", "invalid-report", "report-validation-failed", "timeout",
}
COLLECTOR_FAILURE_REASON_CLASSES = COLLECTOR.COLLECTOR_FAILURE_REASONS
WORKLOAD_FAILURE_REASON_CLASSES = {"insufficient-budget"}
BROWSER_SUMMARY_FAILURE_FIELDS = {
    "test_id", "phase", "status", "error_class", "matcher", "source_file",
    "source_line", "source_column", "source_location_kind",
}
BROWSER_SUMMARY_TEST_IDS = {"cooking-live-review", "cooking-post-operation"}
BROWSER_SUMMARY_FAILURE_PHASES = {"initial", "post-operation"}
BROWSER_SUMMARY_FAILURE_STATUSES = {"failed", "timed_out"}
BROWSER_SUMMARY_ERROR_CLASSES = {"assertion", "timeout", "hook", "runtime", "unknown"}
BROWSER_SUMMARY_MATCHERS = {
    "toBe", "toBeEmpty", "toBeVisible", "toContainText", "toHaveCount", "toHaveURL", "toMatch", "unknown",
}
BROWSER_SUMMARY_LOCATION_KINDS = {"error", "test"}
BROWSER_SUMMARY_SOURCE_RE = re.compile(
    r"^e2e/playwright/cooking-tests/cooking-(?:live-review|post-operation)\.spec\.ts$"
)
RUNTIME_RESULT_EVENTS = {
    "workload-result": "cooking-workload",
    "evidence-scan": "evidence-scan",
    "browser-report-validation": "browser-report-validation",
}
RUNTIME_RESULT_STATUSES = {"passed", "failed"}
RUNTIME_RESULT_PHASES = {"cooking", "evidence"}
RUNTIME_RESULT_REPORT_STATES = {
    "complete", "missing", "partial", "malformed", "truncated", "report-error", "unknown",
}
RUNTIME_RESULT_REASONS = {
    "complete", "invalid-report", "incomplete-phases", "browser-tests-not-passed",
    "timeout", "report-validation-failed", "legacy-unavailable",
}
LEGACY_RUNTIME_PREFIXES = ("cooking ", "runtime ", "worker ")


def _unknown_browser_summary(state: str) -> dict[str, Any]:
    """Return a typed browser state when no usable report is available."""

    return {
        "status": "not-run" if state == "missing" else "unknown",
        "report_state": state,
        "counts": None,
        "observed_phases": [],
        "failure_metadata": [],
    }


def _project_browser_summary(
    root: Path,
    source_records: list[dict[str, Any]],
) -> dict[str, Any]:
    """Project the collector's typed browser summary without error text."""

    records = [record for record in source_records if record.get("label") == "browser-summary"]
    if not records:
        return _unknown_browser_summary("missing")
    if len(records) != 1:
        raise ValueError("browser summary source is duplicated")
    try:
        value = json.loads(_safe_path(root, records[0]["path"]).read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("browser summary is not valid JSON") from error
    if not isinstance(value, dict):
        raise ValueError("browser summary is not an object")

    # ``error`` and ``stack`` are collector-accepted legacy fields, but they
    # are intentionally absent from the public triage projection.
    safe: dict[str, Any] = {}
    for field, item in value.items():
        if field in {"error", "stack"}:
            continue
        if field not in {
            "status", "suite", "test", "phase", "component", "result_origin", "report_state",
            "counts", "observed_phases", "passed_phases", "failure_metadata", "duration_ms", "exit_code", "started_at", "finished_at",
        }:
            raise ValueError("browser summary contains an unknown field")
        if field == "status":
            if item not in BROWSER_SUMMARY_STATUS:
                raise ValueError("browser summary status is invalid")
            safe[field] = item
        elif field == "phase":
            if item not in BROWSER_SUMMARY_PHASES:
                raise ValueError("browser summary phase is invalid")
            safe[field] = item
        elif field == "component":
            if item != "browser-e2e":
                raise ValueError("browser summary component is invalid")
            safe[field] = item
        elif field == "result_origin":
            if item not in BROWSER_SUMMARY_ORIGINS:
                raise ValueError("browser summary origin is invalid")
            safe[field] = item
        elif field == "report_state":
            if item not in BROWSER_SUMMARY_STATES:
                raise ValueError("browser summary report state is invalid")
            safe[field] = item
        elif field in {"suite", "test"}:
            if not isinstance(item, str) or not SAFE_VALUE.fullmatch(item):
                raise ValueError("browser summary identifier is invalid")
            safe[field] = item
        elif field in {"duration_ms", "exit_code"}:
            if type(item) is not int or not 0 <= item <= 2**31 - 1:
                raise ValueError("browser summary number is invalid")
            safe[field] = item
        elif field in {"started_at", "finished_at"}:
            # Timestamps are useful only when they are canonical bounded
            # scalars; malformed legacy text is omitted rather than echoed.
            if isinstance(item, str) and SAFE_VALUE.fullmatch(item):
                safe[field] = item
        elif field == "counts":
            if (
                not isinstance(item, dict)
                or set(item) != BROWSER_SUMMARY_COUNT_FIELDS
                or any(type(number) is not int or not 0 <= number <= 256 for number in item.values())
            ):
                raise ValueError("browser summary counts are invalid")
            safe[field] = dict(item)
        elif field == "observed_phases":
            if (
                not isinstance(item, list)
                or len(item) > len(BROWSER_SUMMARY_PHASE_VALUES)
                or item != sorted(item)
                or any(phase not in BROWSER_SUMMARY_PHASE_VALUES for phase in item)
                or len(set(item)) != len(item)
            ):
                raise ValueError("browser summary observed phases are invalid")
            safe[field] = list(item)
        elif field == "passed_phases":
            if (
                not isinstance(item, list)
                or len(item) > len(BROWSER_SUMMARY_PHASE_VALUES)
                or item != sorted(item)
                or any(phase not in BROWSER_SUMMARY_PHASE_VALUES for phase in item)
                or len(set(item)) != len(item)
            ):
                raise ValueError("browser summary passed phases are invalid")
            safe[field] = list(item)
        elif field == "failure_metadata":
            if not isinstance(item, list) or len(item) > 8:
                raise ValueError("browser summary failure metadata is invalid")
            metadata: list[dict[str, Any]] = []
            for failure in item:
                if not isinstance(failure, dict) or set(failure) != BROWSER_SUMMARY_FAILURE_FIELDS:
                    raise ValueError("browser summary failure metadata entry is invalid")
                if (
                    failure["test_id"] not in BROWSER_SUMMARY_TEST_IDS
                    or failure["phase"] not in BROWSER_SUMMARY_FAILURE_PHASES
                    or failure["status"] not in BROWSER_SUMMARY_FAILURE_STATUSES
                    or failure["error_class"] not in BROWSER_SUMMARY_ERROR_CLASSES
                    or failure["matcher"] not in BROWSER_SUMMARY_MATCHERS
                    or failure["source_location_kind"] not in BROWSER_SUMMARY_LOCATION_KINDS
                    or not isinstance(failure["source_file"], str)
                    or BROWSER_SUMMARY_SOURCE_RE.fullmatch(failure["source_file"]) is None
                    or type(failure["source_line"]) is not int
                    or not 1 <= failure["source_line"] <= 100_000
                    or type(failure["source_column"]) is not int
                    or not 1 <= failure["source_column"] <= 10_000
                ):
                    raise ValueError("browser summary failure metadata values are invalid")
                metadata.append(dict(failure))
            safe[field] = metadata
    safe.setdefault("status", "unknown")
    safe.setdefault("report_state", "unknown")
    safe.setdefault("counts", None)
    safe.setdefault("observed_phases", [])
    safe.setdefault("passed_phases", [])
    safe.setdefault("failure_metadata", [])
    return safe


def _unknown_evidence_scan() -> dict[str, Any]:
    """Describe bundles made before the whole-tree scan result was retained."""

    return {
        "schema": 1,
        "status": "unavailable",
        "rule": "missing-result",
        "file_class": "none",
        "path_sha256": None,
        "checked_files": 0,
        "checked_bytes": 0,
    }


def _project_evidence_scan(
    root: Path,
    source_records: list[dict[str, Any]],
) -> dict[str, Any]:
    """Project the scanner status while excluding paths and raw diagnostics."""

    records = [record for record in source_records if record.get("label") == "evidence-scan"]
    if not records:
        marker_result: dict[str, Any] | None = None
        for record in source_records:
            if record.get("label") not in {"serial", "runtime-log", "runtime-structured", "test-output"}:
                continue
            path = _safe_path(root, record["path"])
            for line in path.read_text(encoding="utf-8").splitlines():
                marker_start = line.find("HEPH_GCP_COOKING ")
                if marker_start >= 0:
                    marker_line = line[marker_start:]
                elif line.startswith(LEGACY_RUNTIME_PREFIXES):
                    marker_line = line
                else:
                    continue
                if "event=evidence-scan" not in marker_line:
                    continue
                fields = dict(FAILURE_PAIR.findall(marker_line))
                if "report_status" not in fields:
                    continue
                expected = {
                    "event", "operation", "phase", "status", "exit_code", "report_status", "rule",
                    "file_class", "path_sha256", "checked_files", "checked_bytes",
                }
                if set(fields) != expected or fields["event"] != "evidence-scan" or fields["operation"] != "evidence-scan":
                    raise ValueError("evidence scan marker fields are invalid")
                if fields["phase"] != "evidence" or fields["status"] not in {"passed", "failed"}:
                    raise ValueError("evidence scan marker outcome is invalid")
                report_status = fields["report_status"]
                if report_status not in {"passed", "failed", "unavailable", "error"}:
                    raise ValueError("evidence scan marker report status is invalid")
                if fields["rule"] not in COLLECTOR.EVIDENCE_SCAN_RULES or fields["file_class"] not in COLLECTOR.EVIDENCE_SCAN_FILE_CLASSES:
                    raise ValueError("evidence scan marker classification is invalid")
                if report_status == "passed" and (
                    fields["rule"] != "none"
                    or fields["file_class"] != "none"
                    or fields["path_sha256"] != "none"
                ):
                    raise ValueError("passed evidence scan marker has failure fields")
                if report_status != "passed" and fields["rule"] == "none":
                    raise ValueError("failed evidence scan marker has no failure rule")
                if report_status in {"unavailable", "error"} and fields["rule"] not in {
                    "scanner-unavailable", "scanner-error", "missing-result", "status-report-unavailable"
                }:
                    raise ValueError("unavailable evidence scan marker has an invalid rule")
                digest = fields["path_sha256"]
                if digest != "none" and COLLECTOR.EVIDENCE_SCAN_DIGEST_RE.fullmatch(digest) is None:
                    raise ValueError("evidence scan marker path digest is invalid")
                numbers: dict[str, int] = {}
                for field, maximum in (
                    ("exit_code", 255),
                    ("checked_files", COLLECTOR.EVIDENCE_SCAN_MAX_FILES),
                    ("checked_bytes", COLLECTOR.EVIDENCE_SCAN_MAX_BYTES),
                ):
                    if not fields[field].isascii() or not fields[field].isdecimal() or int(fields[field]) > maximum:
                        raise ValueError("evidence scan marker count is invalid")
                    numbers[field] = int(fields[field])
                marker_result = {
                    "schema": 1,
                    "status": report_status,
                    "rule": fields["rule"],
                    "file_class": fields["file_class"],
                    "path_sha256": None if digest == "none" else digest,
                    "checked_files": numbers["checked_files"],
                    "checked_bytes": numbers["checked_bytes"],
                    "outer_status": fields["status"],
                    "exit_code": numbers["exit_code"],
                }
        return marker_result or _unknown_evidence_scan()
    if len(records) != 1:
        raise ValueError("evidence scan source is duplicated")
    try:
        value = json.loads(_safe_path(root, records[0]["path"]).read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("evidence scan result is not valid JSON") from error
    if not isinstance(value, dict) or set(value) != COLLECTOR.EVIDENCE_SCAN_FIELDS:
        raise ValueError("evidence scan result contains an unknown field")
    if value.get("schema") != 1 or value.get("status") not in COLLECTOR.EVIDENCE_SCAN_STATUSES:
        raise ValueError("evidence scan result classification is invalid")
    if value.get("rule") not in COLLECTOR.EVIDENCE_SCAN_RULES:
        raise ValueError("evidence scan result rule is invalid")
    if value.get("file_class") not in COLLECTOR.EVIDENCE_SCAN_FILE_CLASSES:
        raise ValueError("evidence scan result file class is invalid")
    path_digest = value.get("path_sha256")
    if path_digest is not None and (
        not isinstance(path_digest, str)
        or COLLECTOR.EVIDENCE_SCAN_DIGEST_RE.fullmatch(path_digest) is None
    ):
        raise ValueError("evidence scan result path digest is invalid")
    checked_files = value.get("checked_files")
    checked_bytes = value.get("checked_bytes")
    if (
        type(checked_files) is not int
        or not 0 <= checked_files <= COLLECTOR.EVIDENCE_SCAN_MAX_FILES
        or type(checked_bytes) is not int
        or not 0 <= checked_bytes <= COLLECTOR.EVIDENCE_SCAN_MAX_BYTES
    ):
        raise ValueError("evidence scan result counts are invalid")
    status = value["status"]
    rule = value["rule"]
    if status == "passed" and (rule != "none" or value["file_class"] != "none" or path_digest is not None):
        raise ValueError("passed evidence scan result has failure fields")
    if status != "passed" and rule == "none":
        raise ValueError("failed evidence scan result has no failure rule")
    if status in {"unavailable", "error"} and rule not in {"scanner-unavailable", "scanner-error", "missing-result"}:
        raise ValueError("unavailable evidence scan result has an invalid rule")
    return {
        "schema": 1,
        "status": status,
        "rule": rule,
        "file_class": value["file_class"],
        # The scanner's path digest is retained as a non-reversible audit
        # handle; no path or scanner error text enters public triage.
        "path_sha256": path_digest,
        "checked_files": checked_files,
        "checked_bytes": checked_bytes,
    }


def _unknown_gate_results() -> dict[str, Any]:
    """Represent legacy or missing gate provenance explicitly."""

    return {"schema": 1, "status": "unavailable", "reason": "missing-result"}


def _project_gate_results(
    root: Path,
    source_records: list[dict[str, Any]],
) -> dict[str, Any]:
    """Project the finalized supervisor sidecar without raw diagnostics."""

    records = [record for record in source_records if record.get("label") == "gate-results"]
    if not records:
        return _unknown_gate_results()
    if len(records) != 1:
        raise ValueError("gate results source is duplicated")
    try:
        value = json.loads(_safe_path(root, records[0]["path"]).read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("gate results source is not valid JSON") from error
    try:
        return COLLECTOR._validate_gate_results(value)
    except COLLECTOR.CollectionError as error:
        raise ValueError("gate results source is invalid") from error


def _project_runtime_results(
    root: Path,
    source_records: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Keep independent harness outcomes, including successful outcomes."""

    results: list[dict[str, Any]] = []
    for record in source_records:
        if record.get("label") not in {"serial", "runtime-log", "runtime-structured", "test-output"}:
            continue
        path = _safe_path(root, record["path"])
        for line in path.read_text(encoding="utf-8").splitlines():
            marker_start = line.find("HEPH_GCP_COOKING ")
            if marker_start >= 0:
                marker_line = line[marker_start:]
            elif line.startswith(LEGACY_RUNTIME_PREFIXES):
                marker_line = line
            else:
                continue
            fields = dict(FAILURE_PAIR.findall(marker_line))
            event = fields.get("event")
            operation = fields.get("operation")
            status = fields.get("status")
            if event not in RUNTIME_RESULT_EVENTS or operation != RUNTIME_RESULT_EVENTS[event]:
                continue
            if status not in RUNTIME_RESULT_STATUSES:
                raise ValueError("runtime result status is invalid")
            phase = fields.get("phase")
            if phase not in RUNTIME_RESULT_PHASES:
                raise ValueError("runtime result phase is invalid")
            result: dict[str, Any] = {
                "source": record["label"],
                "event": event,
                "operation": operation,
                "phase": phase,
                "status": status,
            }
            if "exit_code" in fields:
                if not fields["exit_code"].isdigit() or not 0 <= int(fields["exit_code"]) <= 255:
                    raise ValueError("runtime result exit code is invalid")
                result["exit_code"] = int(fields["exit_code"])
            if event == "browser-report-validation":
                report_state = fields.get("report_state")
                reason = fields.get("reason")
                if report_state is None and reason is None:
                    report_state = "unknown"
                    reason = "legacy-unavailable"
                elif report_state not in RUNTIME_RESULT_REPORT_STATES or reason not in RUNTIME_RESULT_REASONS:
                    raise ValueError("browser validation result is invalid")
                result["report_state"] = report_state
                result["reason"] = reason
            if result not in results:
                results.append(result)
            if len(results) >= 12:
                return results
    return results


def _project_retry(
    root: Path,
    source_records: list[dict[str, Any]],
    attempts: list[dict[str, Any]],
) -> dict[str, Any] | None:
    """Project the terminal retry observation independently of the snapshot.

    The marker is written after the database completion lookup, so it remains
    useful when the periodic lineage snapshot predates that lookup. Correlation
    flags expose that distinction without discarding the marker.
    """

    marker_fields: dict[str, str] | None = None
    for record in source_records:
        if record.get("label") not in {"serial", "runtime-log", "runtime-structured"}:
            continue
        path = _safe_path(root, record["path"])
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.startswith("HEPH_COOKING_RETRY"):
                continue
            try:
                canonical = COLLECTOR._project_retry_marker(line)
            except (AttributeError, COLLECTOR.CollectionError) as error:
                raise ValueError("retry marker is invalid") from error
            tokens = canonical.split()
            parsed = dict(token.split("=", 1) for token in tokens[1:])
            if set(parsed) != set(COLLECTOR.RETRY_FIELD_ORDER):
                raise ValueError("retry marker fields are invalid")
            marker_fields = parsed
    if marker_fields is None:
        return None

    typed: dict[str, Any] = {}
    for field in COLLECTOR.RETRY_FIELD_ORDER:
        value = marker_fields[field]
        if field == "attempt_number" and value != "unknown":
            typed[field] = int(value)
        elif field in {"exit_code", "exit_signal"} and value.isdecimal():
            typed[field] = int(value)
        elif field in {"exit_code", "exit_signal"} and value == "none":
            typed[field] = None
        else:
            typed[field] = value

    snapshot_event_ids = {row.get("event_id") for row in attempts if row.get("event_id")}
    snapshot_attempt_ids = {row.get("attempt_id") for row in attempts if row.get("attempt_id")}
    snapshot_run_ids = {
        row.get("attempt_run_id") for row in attempts if row.get("attempt_run_id")
    }
    same_snapshot_row = any(
        row.get("event_id") == typed["event_id"]
        and typed["attempt_id"] != "unknown"
        and row.get("attempt_id") == typed["attempt_id"]
        and row.get("attempt_run_id") == typed["run_id"]
        for row in attempts
    )
    correlation = {
        "event_id": typed["event_id"] in snapshot_event_ids,
        "attempt_id": typed["attempt_id"] != "unknown" and typed["attempt_id"] in snapshot_attempt_ids,
        "run_id": typed["run_id"] in snapshot_run_ids,
        "same_snapshot_row": same_snapshot_row,
    }
    typed["correlated"] = correlation
    return typed


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


def _timestamp_sort_key(value: str) -> tuple[datetime, int]:
    """Parse Rust OffsetDateTime text, retaining nanoseconds beyond datetime."""

    normalized = re.sub(r"^(\d{4}-\d{2}-\d{2})[ T]", r"\1T", value, count=1)
    normalized = normalized.replace(" Z", "Z", 1)
    normalized = re.sub(r" (?=(?:Z|[+-]\d{2}:\d{2}(?::\d{2})?)$)", "", normalized)
    fraction = re.search(r"\.(\d+)(?=(?:Z|[+-]\d{2}:\d{2}(?::\d{2})?)$)", normalized)
    extra_nanoseconds = 0
    if fraction is not None and len(fraction.group(1)) > 6:
        extra_nanoseconds = int((fraction.group(1)[6:] + "000")[:3])
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError as error:
        raise ValueError("triage timestamp is invalid") from error
    if parsed.tzinfo is None:
        raise ValueError("triage timestamp has no timezone")
    return parsed.astimezone(timezone.utc), extra_nanoseconds


def _latest_attempts(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    latest: dict[str, tuple[tuple[datetime, int, int], dict[str, Any]]] = {}
    for index, row in enumerate(rows):
        key = row.get("attempt_id") or row.get("attempt_run_id")
        if key is None:
            continue
        timestamp = next(
            (row[field] for field in ("terminal_at", "attempt_completed_at", "run_updated_at", "sampled_at", "attempt_created_at", "run_created_at") if row.get(field)),
            "1970-01-01T00:00:00Z",
        )
        parsed, extra_nanoseconds = _timestamp_sort_key(timestamp)
        sort_key = (parsed, extra_nanoseconds, index)
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
    if "component" in fields and fields["component"] not in FAILURE_COMPONENT_VALUES:
        fields.pop("component")
    if "result_origin" in fields and fields["result_origin"] not in FAILURE_ORIGIN_VALUES:
        fields.pop("result_origin")
    if "reason_class" in fields and fields["reason_class"] not in (
        BROWSER_SUMMARY_REASON_CLASSES
        | COLLECTOR_FAILURE_REASON_CLASSES
        | WORKLOAD_FAILURE_REASON_CLASSES
    ):
        fields.pop("reason_class")
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
    def _nonzero(value: Any) -> bool:
        if isinstance(value, int):
            return value > 0
        return isinstance(value, str) and value.isdigit() and int(value) > 0

    has_failure_state = any(
        fields.get(field) in FAILURE_STATUS_VALUES for field in ("status", "test_result")
    ) or "error_class" in fields or _nonzero(exit_code) or _nonzero(fields.get("exit_signal"))
    if not has_failure_state:
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


def _normalize_failure_fields(fields: dict[str, Any]) -> dict[str, Any]:
    """Map collector aliases to the bounded failure schema."""

    if "exit" in fields and "exit_code" not in fields:
        fields["exit_code"] = fields.pop("exit")
    elif "exit" in fields:
        fields.pop("exit")
    test_result = fields.get("test_result")
    if isinstance(test_result, int) and not isinstance(test_result, bool):
        if test_result > 0:
            fields["exit_code"] = test_result
        fields.pop("test_result")
    elif isinstance(test_result, str) and test_result.isdigit():
        numeric_result = int(test_result)
        if numeric_result > 0:
            fields["exit_code"] = numeric_result
        fields.pop("test_result")
    return fields


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
                fields = _normalize_failure_fields(fields)
                if label == "browser-summary" and fields.get("status") not in FAILURE_STATUS_VALUES:
                    continue
                if not any(
                    field in fields
                    for field in ("test", "test_result", "status", "error_class", "exit_code", "exit_signal")
                ):
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
                if any(word.rstrip(":").upper() in {"FAIL", "FAILED"} for word in stripped.split()[1:]):
                    fields["status"] = "failed"
                if "class" in fields:
                    fields["error_class"] = fields.pop("class")
                if "tool" in fields:
                    fields["test"] = fields.pop("tool")
                if fields.get("status", "").isdigit():
                    fields["exit_code"] = fields.pop("status")
                fields = _normalize_failure_fields(fields)
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
        "retry": _project_retry(bundle, records, attempts),
        "browserObservations": _project_browser_observations(bundle, records),
        "browser": _project_browser_summary(bundle, records),
        "evidenceScan": _project_evidence_scan(bundle, records),
        "gateResults": _project_gate_results(bundle, records),
        "runtimeResults": _project_runtime_results(bundle, records),
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
