#!/usr/bin/env python3
"""Project private session-chat negative-process evidence into a safe sidecar.

The input log is private evidence.  This projector emits only fixed,
allowlisted fields and never copies input lines, paths, identifiers, or
diagnostics into the result.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import stat
import sys
import tempfile
from pathlib import Path


SCHEMA = 1
SCENARIO = "session-chat-negative-capability"
GOLDEN_TEST = "bearer_push_starts_run_through_production_bootstrap"
HOST_MARKER = (
    "HEPH_SESSION_CHAT_DENIAL_PROBE host=validated "
    "checks=10 refs=unchanged receives=unchanged"
)
TEST_START = re.compile(rf"^test {re.escape(GOLDEN_TEST)} \.\.\.(?P<tail>.*)$")
RESULT_LINE = re.compile(
    r"^test result: (?P<outcome>ok|FAILED)\. "
    r"(?P<passed>[0-9]+) passed; (?P<failed>[0-9]+) failed; "
    r"(?P<ignored>[0-9]+) ignored; (?P<measured>[0-9]+) measured; "
    r"(?P<filtered>[0-9]+) filtered out; finished in [0-9]+(?:\.[0-9]+)?s$"
)
RUNNING_LINE = re.compile(r"^running (?P<count>[0-9]+) tests?$")
ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*m")

REASONS = frozenset(
    {
        "validated",
        "runner_nonzero",
        "private_log_unreadable",
        "invalid_exit_status",
        "truncated_log",
        "marker_missing",
        "marker_duplicate",
        "marker_malformed",
        "golden_test_missing",
        "golden_test_duplicate",
        "golden_test_failed",
        "golden_test_skipped",
        "golden_test_zero_tests",
        "golden_harness_missing",
        "golden_harness_duplicate",
        "golden_harness_wrong_count",
        "marker_outside_harness",
        "golden_result_missing",
        "golden_result_duplicate",
        "golden_result_malformed",
    }
)


def _failure(reason: str, exit_status: int | None = None) -> dict[str, object]:
    if reason not in REASONS or reason == "validated":
        raise ValueError("invalid negative-summary failure reason")
    result: dict[str, object] = {
        "schema": SCHEMA,
        "scenario": SCENARIO,
        "status": "failed",
        "reason": reason,
    }
    if exit_status is not None:
        result["runner_exit_status"] = exit_status
    return result


def _passed() -> dict[str, object]:
    return {
        "schema": SCHEMA,
        "scenario": SCENARIO,
        "status": "passed",
        "reason": "validated",
        "validated_checks": 10,
        "refs": "unchanged",
        "receives": "unchanged",
        "golden_test": GOLDEN_TEST,
        "golden_test_passes": 1,
        "runner_exit_status": 0,
    }


def _parse_exit_status(value: int | str) -> int | None:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return parsed if 0 <= parsed <= 255 else None


def project(log_path: Path, exit_status: int | str) -> dict[str, object]:
    """Return a typed result without exposing any private log content."""

    parsed_exit = _parse_exit_status(exit_status)
    if parsed_exit is None:
        return _failure("invalid_exit_status")
    try:
        payload = log_path.read_bytes()
    except (OSError, ValueError):
        return _failure("private_log_unreadable", parsed_exit)
    if not payload.endswith(b"\n"):
        return _failure("truncated_log", parsed_exit)
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError:
        return _failure("truncated_log", parsed_exit)

    # Libtest's progress line can be interleaved with --nocapture output and
    # color escapes.  Normalize only for structural matching; never emit it.
    lines = [ANSI_ESCAPE.sub("", line) for line in text.splitlines()]
    marker_prefix = "HEPH_SESSION_CHAT_DENIAL_PROBE host="
    marker_candidates = [
        (index, line[line.index(marker_prefix) :])
        for index, line in enumerate(lines)
        if marker_prefix in line
    ]
    if len(marker_candidates) > 1 or any(
        line.count(marker_prefix) > 1 for line in lines
    ):
        return _failure("marker_duplicate", parsed_exit)
    if marker_candidates and marker_candidates[0][1] != HOST_MARKER:
        return _failure("marker_malformed", parsed_exit)
    if not marker_candidates:
        return _failure("marker_missing", parsed_exit)

    selected = [
        (index, match)
        for index, line in enumerate(lines)
        if (match := TEST_START.match(line))
    ]
    selected_name_lines = [line for line in lines if line.startswith(f"test {GOLDEN_TEST} ")]
    if len(selected_name_lines) > 1 or len(selected) > 1:
        return _failure("golden_test_duplicate", parsed_exit)
    if not selected:
        if any(
            match.group("count") == "0"
            for line in lines
            if (match := RUNNING_LINE.match(line))
        ):
            return _failure("golden_test_zero_tests", parsed_exit)
        if selected_name_lines:
            outcome = selected_name_lines[0].rsplit(" ", 1)[-1]
            return _failure(
                "golden_test_skipped" if outcome == "ignored" else "golden_test_failed",
                parsed_exit,
            )
        return _failure("golden_test_missing", parsed_exit)
    selected_index, selected_match = selected[0]
    selected_tail = selected_match.group("tail").strip()
    if selected_tail == "ignored":
        return _failure("golden_test_skipped", parsed_exit)
    if selected_tail == "FAILED":
        return _failure("golden_test_failed", parsed_exit)

    running_indices = [
        index for index, line in enumerate(lines) if RUNNING_LINE.match(line)
    ]
    if not running_indices:
        return _failure("golden_harness_missing", parsed_exit)
    if len(running_indices) > 1:
        return _failure("golden_harness_duplicate", parsed_exit)
    running_index = running_indices[0]
    running_count = RUNNING_LINE.match(lines[running_index]).group("count")
    if running_count == "0":
        return _failure("golden_test_zero_tests", parsed_exit)
    if running_count != "1":
        return _failure("golden_harness_wrong_count", parsed_exit)

    result_candidates = [
        (index, line) for index, line in enumerate(lines) if line.startswith("test result:")
    ]
    if len(result_candidates) > 1:
        return _failure("golden_result_duplicate", parsed_exit)
    if not result_candidates:
        return _failure("golden_result_missing", parsed_exit)
    result_index, result_line = result_candidates[0]
    result = RESULT_LINE.match(result_line)
    if result is None:
        return _failure("golden_result_malformed", parsed_exit)
    marker_index = marker_candidates[0][0]
    if not (running_index < marker_index < result_index):
        return _failure("marker_outside_harness", parsed_exit)
    if not (running_index < selected_index < result_index):
        return _failure("golden_test_failed", parsed_exit)
    if selected_tail != "ok":
        following = [
            line
            for line in lines[selected_index + 1 : result_index]
            if line and HOST_MARKER not in line
        ]
        completions = [line for line in following if line == "ok"]
        if len(completions) > 1:
            return _failure("golden_test_duplicate", parsed_exit)
        if not completions:
            return _failure("golden_test_failed", parsed_exit)
    if result.group("passed") == "0":
        return _failure("golden_test_zero_tests", parsed_exit)
    if (
        result.group("outcome") != "ok"
        or result.group("failed") != "0"
        or result.group("passed") != "1"
        or result.group("ignored") != "0"
    ):
        return _failure("golden_test_failed", parsed_exit)
    if parsed_exit != 0:
        return _failure("runner_nonzero", parsed_exit)
    return _passed()


def write_atomic(path: Path, value: dict[str, object]) -> None:
    """Write a mode-0600 JSON sidecar atomically in the destination folder."""

    encoded = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
    parent = path.parent
    parent_fd = os.open(parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        fd, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=parent)
        temporary = Path(temporary_name)
        try:
            os.fchmod(fd, stat.S_IRUSR | stat.S_IWUSR)
            with os.fdopen(fd, "wb") as stream:
                stream.write(encoded)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, path)
            os.chmod(path, stat.S_IRUSR | stat.S_IWUSR)
            os.fsync(parent_fd)
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    finally:
        os.close(parent_fd)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--private-log", type=Path, required=True)
    parser.add_argument("--exit-status", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    result = project(args.private_log, args.exit_status)
    write_atomic(args.output, result)
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
