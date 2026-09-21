#!/usr/bin/env python3
"""Project private Playwright JSON reports into a bounded safe summary.

The JSON reporter contains errors, stacks, stdio, and attachment bodies.  This
script reads those fields only long enough to classify a known Cooking test;
the output contains typed metadata and never copies report text or payloads.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
from typing import Any


MAX_REPORT_BYTES = 16 * 1024 * 1024
MAX_FAILURES = 8
MAX_TESTS = 256
MAX_REPORTS = 4
MAX_SPECS = 64
MAX_SUITE_DEPTH = 16
KNOWN_TESTS = {
    (
        "cooking-live-review.spec.ts",
        "cooking release install, mailbox, gateway configure, and binding",
    ): ("cooking-live-review", "initial"),
    (
        "cooking-post-operation.spec.ts",
        "cooking post-operation controls, provenance, recovery, and denial",
    ): ("cooking-post-operation", "post-operation"),
}
KNOWN_FILES = frozenset(file_name for file_name, _ in KNOWN_TESTS)
MATCHERS = frozenset(
    {
        "toBe",
        "toBeEmpty",
        "toBeVisible",
        "toContainText",
        "toHaveCount",
        "toHaveURL",
        "toMatch",
    }
)
MATCHER_RE = re.compile(r"\b(to[A-Z][A-Za-z0-9_]*)\s*\(")
ANSI_RE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
ERROR_CLASSES = frozenset({"assertion", "timeout", "hook", "runtime", "unknown"})
REPORT_STATUSES = frozenset({"passed", "failed", "timedOut", "skipped"})
SAFE_REPORT_STATUSES = frozenset({"passed", "failed", "skipped", "interrupted", "timed_out"})
SESSION_CHAT_TEST_ID = "session_chat_new"
SESSION_CHAT_STAGES = (
    "session_chat_initialize",
    "session_chat_send",
    "session_chat_response",
    "session_chat_second_send",
    "session_chat_second_response",
    "session_chat_reconnect",
)
SAFE_EVENT_KEYS = {
    "run_started": frozenset({"event", "test_count"}),
    "stage": frozenset({"event", "stage_id", "status"}),
    "test": frozenset({"event", "test_id", "status", "duration_ms", "retry"}),
    "run_finished": frozenset({"event", "status", "counts"}),
}


def _bounded_int(value: Any, maximum: int) -> int | None:
    if type(value) is not int or value < 1 or value > maximum:
        return None
    return value


def _canonical_file(value: Any, root_dir: str | None) -> str | None:
    if not isinstance(value, str) or "\x00" in value:
        return None
    normalized = value.replace("\\", "/")
    marker = "/e2e/playwright/cooking-tests/"
    if marker in normalized:
        normalized = "e2e/playwright/cooking-tests/" + normalized.split(marker, 1)[1]
    elif normalized.startswith("cooking-tests/"):
        normalized = "e2e/playwright/" + normalized
    elif normalized in KNOWN_FILES:
        normalized = "e2e/playwright/cooking-tests/" + normalized
    elif root_dir and normalized.startswith(root_dir.rstrip("/") + "/"):
        relative = normalized[len(root_dir.rstrip("/") + "/") :]
        if relative.startswith("cooking-tests/"):
            normalized = "e2e/playwright/" + relative
    path = PurePosixPath(normalized)
    if (
        len(path.parts) != 4
        or path.parts[:3] != ("e2e", "playwright", "cooking-tests")
        or path.parts[3] not in KNOWN_FILES
    ):
        return None
    return path.as_posix()


def _classify(error: dict[str, Any] | None, result_status: str) -> tuple[str, str]:
    if result_status == "timedOut":
        return "timeout", "unknown"
    if not isinstance(error, dict):
        return "unknown", "unknown"
    message = error.get("message") if isinstance(error.get("message"), str) else ""
    stack = error.get("stack") if isinstance(error.get("stack"), str) else ""
    text = ANSI_RE.sub("", message + "\n" + stack)
    matcher = next((candidate for candidate in MATCHER_RE.findall(text) if candidate in MATCHERS), "unknown")
    if matcher != "unknown":
        return "assertion", matcher
    if re.search(r"\b(beforeAll|beforeEach|afterAll|afterEach)\b", text):
        return "hook", "unknown"
    if re.search(r"\b(?:timeout|timed out)\b", text, re.IGNORECASE):
        return "timeout", "unknown"
    return "runtime", "unknown"


def _iter_specs(suites: list[Any]):
    pending = [(suite, 1) for suite in reversed(suites)]
    while pending:
        suite, depth = pending.pop()
        if not isinstance(suite, dict) or depth > MAX_SUITE_DEPTH:
            raise ValueError("suite tree exceeds the bounded report contract")
        specs = suite.get("specs", [])
        children = suite.get("suites", [])
        if not isinstance(specs, list) or not isinstance(children, list):
            raise ValueError("suite tree contains an invalid collection")
        for spec in specs:
            if not isinstance(spec, dict):
                raise ValueError("suite tree contains an invalid spec")
            yield spec
        pending.extend((child, depth + 1) for child in reversed(children))


def _failure(spec: dict[str, Any], test: dict[str, Any], result: dict[str, Any], root_dir: str | None) -> dict[str, Any] | None:
    file_value = spec.get("file")
    file_name = Path(str(file_value)).name if isinstance(file_value, str) else ""
    title = spec.get("title")
    known = KNOWN_TESTS.get((file_name, title))
    if known is None or not isinstance(result, dict):
        return None
    status = result.get("status")
    if status not in {"failed", "timedOut"}:
        return None
    source = result.get("errorLocation")
    location_kind = "error"
    if not isinstance(source, dict):
        source = spec
        location_kind = "test"
    source_file = _canonical_file(source.get("file"), root_dir)
    line = _bounded_int(source.get("line"), 100_000)
    column = _bounded_int(source.get("column"), 10_000)
    if source_file is None or line is None or column is None:
        return None
    error = result.get("error") if isinstance(result.get("error"), dict) else None
    error_class, matcher = _classify(error, status)
    return {
        "test_id": known[0],
        "phase": known[1],
        "status": "timed_out" if status == "timedOut" else "failed",
        "error_class": error_class,
        "matcher": matcher,
        "source_file": source_file,
        "source_line": line,
        "source_column": column,
        "source_location_kind": location_kind,
    }


def _unknown(state: str, origin: str = "playwright-report") -> dict[str, Any]:
    return {
        "status": "not-run" if state == "missing" else "unknown",
        "suite": "cooking-playwright",
        "test": "browser-journey",
        "phase": "browser",
        "component": "browser-e2e",
        "result_origin": "no-browser-report" if state == "missing" else origin,
        "report_state": state,
        "counts": {"passed": 0, "failed": 0, "skipped": 0, "timed_out": 0},
        "observed_phases": [],
        "passed_phases": [],
        "failure_metadata": [],
    }


def _read_report(path: Path) -> tuple[dict[str, Any] | None, str]:
    fd = -1
    try:
        if path.is_symlink() or not path.is_file() or path.parent.is_symlink():
            return None, "malformed"
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode):
            return None, "truncated"
        if metadata.st_size > MAX_REPORT_BYTES:
            return None, "truncated"
        raw = os.read(fd, MAX_REPORT_BYTES + 1)
        if len(raw) > MAX_REPORT_BYTES:
            return None, "truncated"
        value = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return None, "malformed"
    finally:
        if fd >= 0:
            os.close(fd)
    if (
        not isinstance(value, dict)
        or not isinstance(value.get("suites"), list)
        or not isinstance(value.get("errors", []), list)
    ):
        return None, "malformed"
    return value, "complete"


def _read_safe_report(path: Path) -> tuple[list[dict[str, Any]] | None, str]:
    """Read only the fixed-field JSONL stream emitted by the installed UI reporter."""
    fd = -1
    try:
        if path.is_symlink() or not path.is_file() or path.parent.is_symlink():
            return None, "malformed"
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode):
            return None, "truncated"
        if metadata.st_size > MAX_REPORT_BYTES:
            return None, "truncated"
        raw = os.read(fd, MAX_REPORT_BYTES + 1)
        if len(raw) > MAX_REPORT_BYTES:
            return None, "truncated"
        text = raw.decode("utf-8")
    except (OSError, UnicodeDecodeError):
        return None, "malformed"
    finally:
        if fd >= 0:
            os.close(fd)
    records: list[dict[str, Any]] = []
    if not text or text.endswith("\n") is False:
        return None, "malformed"
    for line in text.splitlines():
        if len(records) >= MAX_TESTS:
            return None, "truncated"
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            return None, "malformed"
        if not isinstance(value, dict):
            return None, "malformed"
        event = value.get("event")
        if event not in SAFE_EVENT_KEYS or set(value) != SAFE_EVENT_KEYS[event]:
            return None, "malformed"
        records.append(value)
    return records, "complete"


def _safe_summary(records: list[dict[str, Any]]) -> dict[str, Any] | None:
    """Project one complete session-chat reporter stream without retaining its text."""
    if not records or records[0].get("event") != "run_started" or records[-1].get("event") != "run_finished":
        return None
    if type(records[0].get("test_count")) is not int or records[0]["test_count"] != 1:
        return None

    stage_records = [record for record in records if record.get("event") == "stage"]
    test_records = [record for record in records if record.get("event") == "test"]
    if len(test_records) != 1 or not stage_records or len(stage_records) > len(SESSION_CHAT_STAGES) * 2:
        return None
    stage_ids = [record.get("stage_id") for record in stage_records]
    expected_stage_ids = [stage for stage in SESSION_CHAT_STAGES for _ in (0, 1)]
    if stage_ids != expected_stage_ids[: len(stage_records)] or len(stage_records) % 2:
        return None
    failed_stage = False
    for index in range(0, len(stage_records), 2):
        pending, terminal = stage_records[index : index + 2]
        if pending.get("status") != "pending":
            return None
        if terminal.get("status") == "passed":
            continue
        if terminal.get("status") == "failed" and index + 2 == len(stage_records):
            failed_stage = True
            continue
        return None
    if not failed_stage and len(stage_records) != len(expected_stage_ids):
        return None
    expected_events = ["run_started", *(["stage"] * len(stage_records)), "test", "run_finished"]
    if [record.get("event") for record in records] != expected_events:
        return None

    test = test_records[0]
    if test.get("test_id") != SESSION_CHAT_TEST_ID:
        return None
    if test.get("status") not in SAFE_REPORT_STATUSES:
        return None
    if failed_stage and test["status"] not in {"failed", "timed_out"}:
        return None
    if type(test.get("duration_ms")) is not int or test["duration_ms"] < 0:
        return None
    if type(test.get("retry")) is not int or test["retry"] != 0:
        return None
    finished = records[-1]
    finished_status = finished.get("status")
    if finished_status not in SAFE_REPORT_STATUSES:
        return None
    counts = finished.get("counts")
    if not isinstance(counts, dict) or set(counts) != {"passed", "failed", "skipped", "other"}:
        return None
    if any(type(value) is not int or value < 0 for value in counts.values()):
        return None
    expected_counts = {"passed": 0, "failed": 0, "skipped": 0, "other": 0}
    if test["status"] == "passed":
        expected_counts["passed"] = 1
    elif test["status"] == "failed":
        expected_counts["failed"] = 1
    elif test["status"] == "skipped":
        expected_counts["skipped"] = 1
    else:
        expected_counts["other"] = 1
    # Playwright's safe reporter records a timed-out test as ``timed_out`` but
    # closes the run as ``failed`` with the result in ``other``.  Accept only
    # that exact terminal combination; all other status/count mismatches stay
    # fail-closed.
    terminal_status_matches = finished_status == test["status"] or (
        test["status"] == "timed_out"
        and finished_status == "failed"
        and counts == {"passed": 0, "failed": 0, "skipped": 0, "other": 1}
    )
    if counts != expected_counts or not terminal_status_matches:
        return None

    projected_counts = {
        "passed": counts["passed"],
        "failed": counts["failed"],
        "skipped": counts["skipped"],
        "timed_out": 0,
    }
    if test["status"] == "timed_out":
        projected_counts["timed_out"] = 1
    status = "passed" if test["status"] == "passed" else "failed"
    if test["status"] == "timed_out":
        status = "timed_out"
    elif test["status"] not in {"passed", "failed"}:
        status = "unknown"
    passed_phases = ["initial"] if status == "passed" else []
    return {
        "status": status,
        "suite": "cooking-playwright",
        "test": "browser-journey",
        "phase": "browser",
        "component": "browser-e2e",
        # Keep the established projection enum so the existing collector can
        # consume both the JSON and safe installed-UI reporter paths.
        "result_origin": "playwright-report",
        "report_state": "complete",
        "counts": projected_counts,
        "observed_phases": ["initial"],
        "passed_phases": passed_phases,
        "failure_metadata": [],
    }


def _project_session_chat(root: Path) -> dict[str, Any]:
    if root.is_symlink() or not root.is_dir():
        return _unknown("malformed")
    phase_dirs = sorted(
        path for path in root.glob("browser.*") if path.is_dir() and not path.is_symlink()
    )
    if len(phase_dirs) > MAX_REPORTS:
        return _unknown("truncated")
    logs = [path / "playwright.log" for path in phase_dirs if (path / "playwright.log").exists()]
    # One initial run is the only accepted session-chat evidence. A JSON
    # Playwright report or a retry would make the phase ambiguous.
    if len(phase_dirs) != 1 or len(logs) != 1:
        return _unknown("partial" if phase_dirs else "missing")
    if (phase_dirs[0] / "playwright-report.json").exists():
        return _unknown("partial")
    records, state = _read_safe_report(logs[0])
    if records is None:
        return _unknown(state)
    summary = _safe_summary(records)
    return summary if summary is not None else _unknown("partial")


def project(root: Path, scenario: str = "cooking") -> dict[str, Any]:
    if scenario == "session-chat":
        return _project_session_chat(root)
    if scenario != "cooking":
        return _unknown("malformed")
    if root.is_symlink() or not root.is_dir():
        return _unknown("malformed")
    reports = sorted(root.glob("browser.*/playwright-report.json"))
    phase_dirs = sorted(
        path for path in root.glob("browser.*") if path.is_dir() and not path.is_symlink()
    )
    if len(reports) > MAX_REPORTS or len(phase_dirs) > MAX_REPORTS:
        return _unknown("truncated")
    if not reports:
        return _unknown("partial" if phase_dirs else "missing")
    summaries: list[tuple[dict[str, Any], str]] = []
    for report in reports:
        value, state = _read_report(report)
        if value is None:
            return _unknown("partial" if len(reports) > 1 or len(phase_dirs) > 1 else state)
        if value.get("errors"):
            return _unknown("report-error")
        summaries.append((value, state))
    if len(reports) < len(phase_dirs):
        return _unknown("partial")
    counts = {"passed": 0, "failed": 0, "skipped": 0, "timed_out": 0}
    failures: list[dict[str, Any]] = []
    observed_phases: set[str] = set()
    passed_phases: set[str] = set()
    root_dir: str | None = None
    total_specs = 0
    total_tests = 0
    for value, _ in summaries:
        config = value.get("config")
        if isinstance(config, dict) and isinstance(config.get("rootDir"), str):
            root_dir = config["rootDir"]
        try:
            specs = list(_iter_specs(value["suites"]))
        except ValueError:
            return _unknown("partial")
        total_specs += len(specs)
        if total_specs > MAX_SPECS:
            return _unknown("truncated")
        for spec in specs:
            if not isinstance(spec.get("tests"), list):
                return _unknown("partial")
            total_tests += len(spec["tests"])
            if total_tests > MAX_TESTS:
                return _unknown("truncated")
            for test in spec["tests"]:
                if not isinstance(test, dict) or not isinstance(test.get("results"), list):
                    return _unknown("partial")
                if not test["results"] or any(not isinstance(result, dict) for result in test["results"]):
                    return _unknown("partial")
                result = test["results"][-1]
                status = result.get("status")
                if status not in REPORT_STATUSES:
                    return _unknown("partial")
                known = KNOWN_TESTS.get(
                    (Path(str(spec.get("file"))).name, spec.get("title"))
                )
                if known is not None:
                    observed_phases.add(known[1])
                    if status == "passed":
                        passed_phases.add(known[1])
                if status == "passed":
                    counts["passed"] += 1
                elif status == "failed":
                    counts["failed"] += 1
                elif status == "timedOut":
                    counts["timed_out"] += 1
                elif status == "skipped":
                    counts["skipped"] += 1
                item = _failure(spec, test, result, root_dir)
                if item is not None and len(failures) < MAX_FAILURES:
                    failures.append(item)
    if counts["timed_out"]:
        status = "timed_out"
    elif counts["failed"]:
        status = "failed"
    elif counts["passed"]:
        status = "passed"
    else:
        status = "unknown"
    return {
        "status": status,
        "suite": "cooking-playwright",
        "test": "browser-journey",
        "phase": "browser",
        "component": "browser-e2e",
        "result_origin": "playwright-report",
        "report_state": "complete",
        "counts": counts,
        "observed_phases": sorted(observed_phases),
        "passed_phases": sorted(passed_phases),
        "failure_metadata": failures,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence_root", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--scenario", choices=("cooking", "session-chat"), default="cooking")
    parser.add_argument("--require-complete-journey", action="store_true")
    args = parser.parse_args()
    summary = project(args.evidence_root, args.scenario)
    encoded = (json.dumps(summary, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
    if len(encoded) > 64 * 1024:
        raise SystemExit("projected browser summary exceeds limit")
    args.output.write_bytes(encoded)
    args.output.chmod(0o600)
    if not args.require_complete_journey:
        return 0
    if summary["report_state"] != "complete":
        return 2
    expected_phases = ["initial"] if args.scenario == "session-chat" else ["initial", "post-operation"]
    if summary["observed_phases"] != expected_phases:
        return 3
    if summary["passed_phases"] != expected_phases or summary["status"] != "passed":
        return 4
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
