#!/usr/bin/env python3
"""Record and validate bounded monotonic Cooking CI phase timings.

The raw JSONL file is private run state.  ``project`` emits the smaller safe
projection used by diagnostics.  Workload records are explicitly marked
informational; they are useful for locating cost, but never carry an
acceptance decision.  ``import-markers`` accepts only the fixed Rust stderr
marker grammar and adds those measurements as workload records.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from pathlib import Path
from typing import Any

SCHEMA = 1
MAX_RECORD_BYTES = 16 * 1024
MAX_RECORDS = 512
MAX_DURATION_MS = 45 * 60 * 1000
MAX_COUNTER = 1_000_000_000_000
MAX_GENERATION = 9_223_372_036_854_775_807
SHA40 = re.compile(r"[0-9a-f]{40}\Z")
SHA64 = re.compile(r"[0-9a-f]{64}\Z")
FINGERPRINT = re.compile(r"[0-9a-f]{32}\Z")
RUN_ID = re.compile(r"(?:manual|[0-9]+)\Z")
RUST_MARKER = re.compile(
    r"HEPH_GCP_COOKING event=phase-timing phase=([a-z0-9-]+) "
    r"status=(passed|failed|timed-out|cancelled) duration_ms=([0-9]+)\Z"
)

PHASE_ORDER = (
    "preflight-quota",
    "preflight-cache",
    "vm-create",
    "vm-wait",
    "startup-metadata",
    "startup-runner-image",
    "startup-host-packages",
    "startup-accounts",
    "startup-passt",
    "startup-cgroup",
    "startup-apparmor",
    "startup-rust-toolchain",
    "startup-libkrunfw",
    "startup-libkrun",
    "startup-checkout",
    "cooking-supervisor",
    "dependency-setup",
    "cache-download",
    "cache-extract",
    "workflow-images",
    "browser-setup",
    "metadata-guard",
    "project-build",
    "production-project-build",
    "runtime-guest-build",
    "runtime-worker-build",
    "runtime-smoke",
    "gateway-edge-ready",
    "gateway-services-ready",
    "gateway-readiness",
    "oci-image-materialization",
    "oci-builder",
    "oci-verifier",
    "golden-tests",
    "database-tests",
    "browser-initial",
    "browser-post-operation",
    "evidence-scan",
    "archive",
    "upload",
    "vm-delete",
    "post-delete-download",
    "cleanup-verification",
)
PHASES = set(PHASE_ORDER)
# PHASE_ORDER is the vocabulary order only.  Timings may be nested or emitted
# by concurrent processes, so validation orders timestamps within each clock
# stream rather than imposing this list as a nesting order.
TRUST = {"supervisor", "workload"}
CLOCK_DOMAINS = {
    "controller",
    "guest-startup",
    "guest-runtime",
    "workload",
    "workload-libkrun",
    "workload-gateway",
    "collection",
}
TIMING_DIAGNOSTIC_STAGES = {
    "workload-source",
    "supervisor-source",
    "workload-validation",
    "supervisor-validation",
    "marker-import",
    "projection",
    "final-projection",
    "upload",
}
TIMING_DIAGNOSTIC_REASONS = {
    "missing-source",
    "record-read",
    "record-write",
    "invalid",
    "duplicate",
    "identity",
    "unclosed-start",
    "unmatched-end",
    "end-before-start",
    "incomplete",
    "ordering",
    "missing-phase",
    "trust",
    "clock-domain",
    "path",
    "unknown",
}
SUPERVISOR_PHASE_DOMAINS = {
    "archive": {"guest-startup"},
    "evidence-scan": {"guest-startup"},
    "upload": {"guest-startup"},
}
WORKLOAD_PHASE_DOMAINS = {
    "dependency-setup": {"workload"},
    "project-build": {"workload"},
    # The production proof is emitted by golden.rs and imported from the
    # libkrun workload marker stream.  Keep the host project-build span
    # separate so overlapping durations are never silently combined.
    "production-project-build": {"workload-libkrun"},
    "browser-setup": {"workload", "workload-libkrun"},
    "runtime-guest-build": {"workload-libkrun"},
    "runtime-worker-build": {"workload-libkrun"},
    "oci-image-materialization": {"workload-libkrun"},
    "gateway-edge-ready": {"workload-gateway"},
    "gateway-services-ready": {"workload-libkrun"},
    "gateway-readiness": {"workload-libkrun"},
    "oci-builder": {"workload-libkrun"},
    "oci-verifier": {"workload-libkrun"},
    "golden-tests": {"workload-libkrun"},
    "database-tests": {"workload-libkrun"},
    "browser-initial": {"workload-libkrun"},
    "browser-post-operation": {"workload-libkrun"},
}
OUTCOMES = {"passed", "failed", "timed-out", "cancelled", "skipped", "unknown"}
CACHE_STATES = {"hit", "miss", "not-applicable"}
COMMON_KEYS = {
    "schema",
    "record",
    "phase",
    "trust",
    "clock_domain",
    "mono_ns",
    "outcome",
    "run_id",
    "attempt",
    "occurrence",
    "source_sha",
    "image_fingerprint",
    "cache_sha256",
    "cache_generation",
    "cache_state",
    "bytes",
    "count",
    "test_count",
    "passed_test_count",
    "ignored_test_count",
}
IDENTITY_KEYS = {
    "schema",
    "phase",
    "trust",
    "clock_domain",
    "run_id",
    "attempt",
    "occurrence",
    "source_sha",
    "image_fingerprint",
    "cache_sha256",
    "cache_generation",
    "cache_state",
}


class TimingError(ValueError):
    """A malformed or unsafe timing record with bounded pair context."""

    def __init__(
        self,
        message: str,
        *,
        phase: str | None = None,
        clock_domain: str | None = None,
        occurrence: int = 0,
    ) -> None:
        super().__init__(message)
        self.phase = phase
        self.clock_domain = clock_domain
        self.occurrence = occurrence


def fail(message: str) -> "NoReturn":
    raise TimingError(message)


def fail_at(message: str, value: dict[str, Any]) -> "NoReturn":
    """Raise a timing error with only the current record's safe pair fields."""

    raise TimingError(
        message,
        phase=value.get("phase") if isinstance(value.get("phase"), str) else None,
        clock_domain=value.get("clock_domain") if isinstance(value.get("clock_domain"), str) else None,
        occurrence=value.get("occurrence") if type(value.get("occurrence")) is int else 0,
    )


def timing_error_class(error: Exception) -> str:
    """Map helper failures to a fixed diagnostic class without echoing input."""

    message = str(error).lower()
    if "duplicate" in message:
        return "duplicate"
    if any(token in message for token in ("run_id", "attempt", "source_sha", "fingerprint", "identity")):
        return "identity"
    if "required phase is missing" in message:
        return "missing-phase"
    if "trust" in message:
        return "trust"
    if "clock domain" in message:
        return "clock-domain"
    if "contains an incomplete phase" in message:
        return "unclosed-start"
    if "no matching start" in message:
        return "unmatched-end"
    if "precedes phase start" in message:
        return "end-before-start"
    if "incomplete" in message:
        return "incomplete"
    if "monotonic order" in message:
        return "ordering"
    if "path" in message or "symlink" in message:
        return "path"
    if any(token in message for token in ("read", "decoded", "json")):
        return "record-read"
    if any(token in message for token in ("writ", "size bound", "count exceeds")):
        return "record-write"
    if "invalid" in message or "unsupported" in message or "schema" in message:
        return "invalid"
    return "unknown"


def shell_timing_error_class(error: Exception) -> str:
    """Keep the legacy shell failure marker reason vocabulary stable."""

    reason = timing_error_class(error)
    if reason in {"identity", "path", "record-read", "record-write"}:
        return reason
    return "pair"


def bounded_int(value: Any, name: str) -> int:
    if type(value) is not int or value < 0 or value > MAX_COUNTER:
        fail(f"{name} is outside its bounded integer range")
    return value


def validate_common(value: dict[str, Any], *, record: str) -> None:
    if set(value) - COMMON_KEYS:
        fail("record contains an unsupported field")
    if type(value.get("schema")) is not int or value["schema"] != SCHEMA or value.get("record") != record:
        fail("record schema or record type is invalid")
    for name, allowed in (("phase", PHASES), ("trust", TRUST), ("clock_domain", CLOCK_DOMAINS)):
        item = value.get(name)
        if not isinstance(item, str) or item not in allowed:
            fail(f"{name} is invalid")
    if type(value.get("mono_ns")) is not int or not 0 <= value["mono_ns"] <= 10**20:
        fail("monotonic timestamp is invalid")
    run_id = value.get("run_id")
    if not isinstance(run_id, str) or RUN_ID.fullmatch(run_id) is None:
        fail("run_id is invalid")
    attempt = value.get("attempt")
    if type(attempt) is not int or not 1 <= attempt <= MAX_COUNTER:
        fail("attempt is invalid")
    occurrence = value.get("occurrence")
    if type(occurrence) is not int or not 1 <= occurrence <= MAX_COUNTER:
        fail("occurrence is invalid")
    source_sha = value.get("source_sha")
    if not isinstance(source_sha, str) or SHA40.fullmatch(source_sha) is None:
        fail("source_sha is invalid")
    if "image_fingerprint" in value and (
        not isinstance(value["image_fingerprint"], str)
        or FINGERPRINT.fullmatch(value["image_fingerprint"]) is None
    ):
        fail("image_fingerprint is invalid")
    if "cache_sha256" in value and (
        not isinstance(value["cache_sha256"], str) or SHA64.fullmatch(value["cache_sha256"]) is None
    ):
        fail("cache_sha256 is invalid")
    if "cache_generation" in value:
        generation = value["cache_generation"]
        if type(generation) is not int or not 1 <= generation <= MAX_GENERATION:
            fail("cache_generation is invalid")
    if "cache_state" in value and (
        not isinstance(value["cache_state"], str) or value["cache_state"] not in CACHE_STATES
    ):
        fail("cache_state is invalid")
    for name in (
        "bytes",
        "count",
        "test_count",
        "passed_test_count",
        "ignored_test_count",
    ):
        if name in value:
            bounded_int(value[name], name)


def validate_record(value: Any, *, record: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail("record is not an object")
    validate_common(value, record=record)
    if record == "end":
        outcome = value.get("outcome")
        if not isinstance(outcome, str) or outcome not in OUTCOMES:
            fail("outcome is invalid")
    return value


def path_for(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute() or path.is_symlink():
        fail("timing path must be an absolute non-symlink path")
    if path.exists() and not path.is_file():
        fail("timing path must be a regular file")
    if any(parent.is_symlink() for parent in path.parents):
        fail("timing path parents must not be symlinks")
    return path


def read_records(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    records: list[dict[str, Any]] = []
    try:
        with path.open(encoding="utf-8") as handle:
            line_number = 0
            while line := handle.readline(MAX_RECORD_BYTES + 1):
                line_number += 1
                if len(line.encode("utf-8")) > MAX_RECORD_BYTES:
                    fail(f"record {line_number} exceeds the size bound")
                try:
                    value = json.loads(line)
                except (TypeError, ValueError):
                    fail(f"record {line_number} is not valid JSON")
                if not isinstance(value, dict):
                    fail(f"record {line_number} is not an object")
                record_type = value.get("record")
                if record_type not in {"start", "end"}:
                    fail(f"record {line_number} has an invalid type")
                records.append(validate_record(value, record=record_type))
                if len(records) > MAX_RECORDS:
                    fail("timing record count exceeds the bound")
    except UnicodeError:
        fail("timing file cannot be decoded")
    except OSError as exc:
        fail(f"timing file cannot be read: {exc}")
    return records


def timing_diagnostic(
    path: Path,
    *,
    stage: str,
    required: set[str],
    required_trust: str | None = None,
    expected_run_id: str | None = None,
    expected_attempt: int | None = None,
    expected_source_sha: str | None = None,
    expected_image_fingerprint: str | None = None,
) -> dict[str, Any]:
    """Return a closed diagnostic summary while never returning source text.

    This deliberately has a successful process exit: it is called after a
    failed timing operation and must not replace the workload or collector
    result.  Phase names are admitted only from ``PHASES`` and all output
    fields have bounded vocabularies/counts.
    """

    if stage not in TIMING_DIAGNOSTIC_STAGES:
        raise TimingError("timing diagnostic stage is invalid")
    safe_required = required & PHASES
    available: set[str] = set()
    reason = "unknown"
    records: list[dict[str, Any]] = []
    failed_phase = "none"
    failed_clock_domain = "none"
    failed_occurrence = 0
    if path.is_symlink() or any(parent.is_symlink() for parent in path.parents):
        reason = "path"
    elif not path.exists():
        reason = "missing-source"
    elif not path.is_file():
        reason = "path"
    else:
        try:
            records = read_records(path)
            available = {record["phase"] for record in records}
            validate_pairs(
                records,
                required=safe_required,
                required_trust=required_trust,
                expected_run_id=expected_run_id,
                expected_attempt=expected_attempt,
                expected_source_sha=expected_source_sha,
                expected_image_fingerprint=expected_image_fingerprint,
            )
        except (TimingError, OSError, TypeError, ValueError) as error:
            reason = timing_error_class(error)
            if isinstance(error, TimingError):
                # Emit pair context atomically: partial context would be
                # ambiguous after the safe collector strips the raw record.
                if (
                    error.phase in PHASES
                    and error.clock_domain in CLOCK_DOMAINS
                    and type(error.occurrence) is int
                    and 1 <= error.occurrence <= MAX_COUNTER
                ):
                    failed_phase = error.phase
                    failed_clock_domain = error.clock_domain
                    failed_occurrence = error.occurrence
            # A valid prefix is useful even when a later line fails.  Read it
            # through a bounded JSON pass and admit only known phase names.
            try:
                total_bytes = 0
                with path.open(encoding="utf-8") as handle:
                    for line_number in range(MAX_RECORDS):
                        line = handle.readline(MAX_RECORD_BYTES + 1)
                        if not line:
                            break
                        line_bytes = len(line.encode("utf-8"))
                        total_bytes += line_bytes
                        if line_bytes > MAX_RECORD_BYTES or total_bytes > MAX_RECORDS * MAX_RECORD_BYTES:
                            if reason == "unknown":
                                reason = "record-write"
                            break
                        try:
                            value = json.loads(line)
                        except (TypeError, ValueError):
                            continue
                        phase = value.get("phase") if isinstance(value, dict) else None
                        if isinstance(phase, str) and phase in PHASES:
                            available.add(phase)
                        if len(available) >= len(PHASES):
                            break
            except (OSError, UnicodeError):
                if reason == "unknown":
                    reason = "record-read"
    ordered_available = [phase for phase in PHASE_ORDER if phase in available]
    ordered_missing = [phase for phase in PHASE_ORDER if phase in safe_required - available]
    if reason not in TIMING_DIAGNOSTIC_REASONS:
        reason = "unknown"
    return {
        "event": "phase-timing",
        "status": "unavailable",
        "failed_stage": stage,
        "reason_class": reason,
        "failed_phase": failed_phase,
        "failed_clock_domain": failed_clock_domain,
        "failed_occurrence": failed_occurrence,
        "available_count": len(ordered_available),
        "available_phases": ",".join(ordered_available) or "none",
        "missing_count": len(ordered_missing),
        "missing_phases": ",".join(ordered_missing) or "none",
    }


def write_record(path: Path, value: dict[str, Any]) -> None:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n"
    if len(encoded.encode("utf-8")) > MAX_RECORD_BYTES:
        fail("timing record exceeds the size bound")
    try:
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        if any(parent.is_symlink() for parent in path.parents):
            fail("timing path parents must not be symlinks")
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW, 0o600)
        try:
            with os.fdopen(descriptor, "a", encoding="utf-8") as handle:
                descriptor = -1
                handle.write(encoded)
        finally:
            if descriptor >= 0:
                os.close(descriptor)
    except OSError as exc:
        fail(f"timing record cannot be written: {exc}")


def identity(args: argparse.Namespace) -> dict[str, Any]:
    source_sha = args.source_sha or os.environ.get("HEPH_GCP_PHASE_TIMING_SOURCE_SHA", "")
    if SHA40.fullmatch(source_sha) is None:
        fail("source_sha must be supplied as a 40-character lowercase SHA")
    run_id = args.run_id or os.environ.get("HEPH_GCP_PHASE_TIMING_RUN_ID", "manual")
    attempt = args.attempt or int(os.environ.get("HEPH_GCP_PHASE_TIMING_ATTEMPT", "1"))
    occurrence = args.occurrence or 1
    if RUN_ID.fullmatch(run_id) is None or not 1 <= attempt <= MAX_COUNTER:
        fail("run identity is invalid")
    if not 1 <= occurrence <= MAX_COUNTER:
        fail("occurrence is invalid")
    result: dict[str, Any] = {
        "run_id": run_id,
        "attempt": attempt,
        "occurrence": occurrence,
        "source_sha": source_sha,
    }
    image = args.image_fingerprint or os.environ.get("HEPH_GCP_PHASE_TIMING_IMAGE_FINGERPRINT")
    if image:
        result["image_fingerprint"] = image
    if args.cache_sha256:
        result["cache_sha256"] = args.cache_sha256
    if args.cache_generation is not None:
        result["cache_generation"] = args.cache_generation
    if args.cache_state:
        result["cache_state"] = args.cache_state
    return result


def validate_pairs(
    records: list[dict[str, Any]],
    *,
    required: set[str] | None = None,
    required_trust: str | None = None,
    expected_run_id: str | None = None,
    expected_attempt: int | None = None,
    expected_source_sha: str | None = None,
    expected_image_fingerprint: str | None = None,
    required_supervisor: set[str] | None = None,
    required_workload: set[str] | None = None,
) -> list[dict[str, Any]]:
    starts: dict[tuple[str, str, str, int], dict[str, Any]] = {}
    seen_starts: set[tuple[str, str, str, int]] = set()
    complete: list[dict[str, Any]] = []
    previous: dict[tuple[str, str], int] = {}
    identity_values: tuple[Any, ...] | None = None
    for value in records:
        if required_trust is not None and value["trust"] != required_trust:
            fail_at("timing record trust does not match the input origin", value)
        identity = (
            value["run_id"],
            value["attempt"],
            value["source_sha"],
            value.get("image_fingerprint"),
        )
        if identity_values is None:
            identity_values = identity
        elif identity != identity_values:
            fail_at("timing record identity changed within the run", value)
        if expected_run_id is not None and value["run_id"] != expected_run_id:
            fail_at("timing run_id does not match the expected run", value)
        if expected_attempt is not None and value["attempt"] != expected_attempt:
            fail_at("timing attempt does not match the expected attempt", value)
        if expected_source_sha is not None and value["source_sha"] != expected_source_sha:
            fail_at("timing source_sha does not match the expected workload", value)
        if expected_image_fingerprint and value.get("image_fingerprint") != expected_image_fingerprint:
            fail_at("timing image fingerprint does not match the expected image", value)
        key = (value["phase"], value["trust"], value["clock_domain"], value["occurrence"])
        stream = (value["trust"], value["clock_domain"])
        if value["record"] == "start":
            if key in starts or key in seen_starts:
                fail_at("duplicate phase start", value)
            starts[key] = value
            seen_starts.add(key)
            if value["mono_ns"] < previous.get(stream, 0):
                fail_at("phase starts are out of monotonic order", value)
            previous[stream] = value["mono_ns"]
            continue
        start = starts.pop(key, None)
        if start is None:
            fail_at("phase end has no matching start", value)
        if value["mono_ns"] < start["mono_ns"]:
            fail_at("phase end precedes phase start", value)
        if value["mono_ns"] - start["mono_ns"] > MAX_DURATION_MS * 1_000_000:
            fail_at("phase duration exceeds the bound", value)
        if any(start.get(name) != value.get(name) for name in IDENTITY_KEYS):
            fail_at("phase provenance changed between start and end", value)
        item = dict(start)
        item.update(value)
        item["duration_ms"] = (value["mono_ns"] - start["mono_ns"]) // 1_000_000
        complete.append(item)
        previous[stream] = value["mono_ns"]
    if starts:
        fail_at("timing file contains an incomplete phase", next(iter(starts.values())))
    if required:
        observed = {item["phase"] for item in complete}
        missing = required - observed
        if missing:
            fail("required phase is missing")
    for trust, phases in (("supervisor", required_supervisor or set()), ("workload", required_workload or set())):
        for phase in phases:
            matches = [item for item in complete if item["phase"] == phase and item["trust"] == trust]
            if not matches:
                raise TimingError("required phase trust is missing", phase=phase)
            expected_domains = (
                SUPERVISOR_PHASE_DOMAINS.get(phase)
                if trust == "supervisor"
                else WORKLOAD_PHASE_DOMAINS.get(phase)
            ) or (
                {"guest-startup", "guest-runtime", "controller", "collection"}
                if trust == "supervisor"
                else {"workload", "workload-libkrun", "workload-gateway"}
            )
            if any(item["clock_domain"] not in expected_domains for item in matches):
                item = next(item for item in matches if item["clock_domain"] not in expected_domains)
                fail_at("required phase clock domain is invalid", item)
    return complete


def validate_projection(
    value: Any,
    *,
    required: set[str] | None = None,
    expected_run_id: str | None = None,
    expected_attempt: int | None = None,
    expected_source_sha: str | None = None,
    expected_image_fingerprint: str | None = None,
    required_supervisor: set[str] | None = None,
    required_workload: set[str] | None = None,
) -> dict[str, Any]:
    if not isinstance(value, dict) or value.get("schema") != SCHEMA or not isinstance(value.get("phases"), list):
        fail("timing projection schema is invalid")
    phases = value["phases"]
    if len(phases) > MAX_RECORDS // 2:
        fail("timing projection count exceeds the bound")
    records: list[dict[str, Any]] = []
    cursors: dict[tuple[str, str], int] = {}
    required_projection_keys = {
        "phase",
        "trust",
        "measurement",
        "clock_domain",
        "duration_ms",
        "outcome",
        "run_id",
        "attempt",
        "occurrence",
        "source_sha",
    }
    for phase in phases:
        if not isinstance(phase, dict) or set(phase) - (COMMON_KEYS - {"record", "mono_ns"} | {"duration_ms", "measurement"}):
            fail("timing projection contains an unsupported field")
        if not required_projection_keys <= set(phase):
            fail("timing projection is missing a required field")
        if not isinstance(phase.get("phase"), str) or phase["phase"] not in PHASES:
            fail("timing projection phase is invalid")
        if not isinstance(phase.get("measurement"), str) or phase["measurement"] not in {"trusted", "informational"}:
            fail("timing projection measurement is invalid")
        trust = phase.get("trust")
        expected_measurement = "trusted" if trust == "supervisor" else "informational" if trust == "workload" else None
        if expected_measurement != phase["measurement"]:
            fail("timing projection trust is invalid")
        bounded_int(phase.get("duration_ms"), "duration_ms")
        if phase["duration_ms"] > MAX_DURATION_MS:
            fail("timing projection duration exceeds the bound")
        record = dict(phase)
        record.pop("measurement", None)
        record.pop("duration_ms", None)
        stream = (record["trust"], record["clock_domain"])
        start_ns = cursors.get(stream, 0)
        record.update({"schema": SCHEMA, "record": "start", "mono_ns": start_ns})
        records.append(record)
        record_end = dict(record)
        record_end["record"] = "end"
        record_end["outcome"] = phase.get("outcome")
        record_end["mono_ns"] = start_ns + phase["duration_ms"] * 1_000_000
        records.append(record_end)
        cursors[stream] = record_end["mono_ns"] + 1
    validate_pairs(
        records,
        required=required,
        expected_run_id=expected_run_id,
        expected_attempt=expected_attempt,
        expected_source_sha=expected_source_sha,
        expected_image_fingerprint=expected_image_fingerprint,
        required_supervisor=required_supervisor,
        required_workload=required_workload,
    )
    return value


def projection(complete: list[dict[str, Any]]) -> dict[str, Any]:
    phases = []
    for item in complete:
        safe = {
            "phase": item["phase"],
            "trust": item["trust"],
            "measurement": "trusted" if item["trust"] == "supervisor" else "informational",
            "clock_domain": item["clock_domain"],
            "duration_ms": item["duration_ms"],
            "outcome": item["outcome"],
        }
        for name in (
            "run_id",
            "attempt",
            "occurrence",
            "source_sha",
            "image_fingerprint",
            "cache_sha256",
            "cache_generation",
            "cache_state",
            "bytes",
            "count",
            "test_count",
            "passed_test_count",
            "ignored_test_count",
        ):
            if name in item:
                safe[name] = item[name]
        phases.append(safe)
    return {"schema": SCHEMA, "phases": phases}


def import_markers(args: argparse.Namespace) -> int:
    source = path_for(args.input)
    output = path_for(args.output)
    if source == output:
        fail("marker input and timing output must be different files")
    source_sha = args.source_sha
    if SHA40.fullmatch(source_sha) is None:
        fail("source_sha must be supplied as a 40-character lowercase SHA")
    run_id = args.run_id or os.environ.get("HEPH_GCP_PHASE_TIMING_RUN_ID", "manual")
    attempt = args.attempt or int(os.environ.get("HEPH_GCP_PHASE_TIMING_ATTEMPT", "1"))
    image_fingerprint = args.image_fingerprint or os.environ.get("HEPH_GCP_PHASE_TIMING_IMAGE_FINGERPRINT")
    if image_fingerprint and FINGERPRINT.fullmatch(image_fingerprint) is None:
        fail("image fingerprint is invalid")
    if RUN_ID.fullmatch(run_id) is None or not 1 <= attempt <= MAX_COUNTER:
        fail("run identity is invalid")
    existing = read_records(output)
    validate_pairs(existing, required_trust="workload")
    marker_rows: list[tuple[str, str, int]] = []
    try:
        with source.open(encoding="utf-8") as handle:
            line_number = 0
            while line := handle.readline(MAX_RECORD_BYTES + 1):
                line_number += 1
                if len(line.encode("utf-8")) > MAX_RECORD_BYTES:
                    fail(f"marker input line {line_number} exceeds the size bound")
                text = line.rstrip("\r\n")
                if "HEPH_GCP_COOKING event=phase-timing" not in text:
                    continue
                match = RUST_MARKER.fullmatch(text)
                if match is None:
                    fail(f"marker input line {line_number} is malformed")
                phase, outcome, duration_text = match.groups()
                duration_ms = int(duration_text)
                if phase not in PHASES or duration_ms > MAX_DURATION_MS:
                    fail(f"marker input line {line_number} has an invalid value")
                marker_rows.append((phase, outcome, duration_ms))
                if len(existing) + (len(marker_rows) * 2) > MAX_RECORDS:
                    fail("timing record count exceeds the bound")
    except UnicodeError:
        fail("marker input cannot be decoded")
    except OSError as exc:
        fail(f"marker input cannot be read: {exc}")
    occurrence_by_phase: dict[tuple[str, str], int] = {}
    for record in existing:
        if record["trust"] == "workload":
            key = (record["phase"], args.clock_domain)
            occurrence_by_phase[key] = max(occurrence_by_phase.get(key, 0), record["occurrence"])
    cursor = max((record["mono_ns"] for record in existing), default=0) + 1
    cursor = max(cursor, time.monotonic_ns())
    generated: list[dict[str, Any]] = []
    for phase, outcome, duration_ms in marker_rows:
        key = (phase, args.clock_domain)
        occurrence = occurrence_by_phase.get(key, 0) + 1
        occurrence_by_phase[key] = occurrence
        common = {
            "schema": SCHEMA,
            "trust": "workload",
            "clock_domain": args.clock_domain,
            "run_id": run_id,
            "attempt": attempt,
            "occurrence": occurrence,
            "source_sha": source_sha,
        }
        if image_fingerprint:
            common["image_fingerprint"] = image_fingerprint
        start = {"record": "start", "phase": phase, "mono_ns": cursor, **common}
        end = {
            "record": "end",
            "phase": phase,
            "mono_ns": cursor + duration_ms * 1_000_000,
            "outcome": outcome,
            **common,
        }
        generated.extend((start, end))
        cursor = end["mono_ns"] + 1
    validate_pairs(existing + generated, required_trust="workload")
    for record in generated:
        write_record(output, record)
    return 0


def parser() -> argparse.ArgumentParser:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--path", required=True)
    common.add_argument("--phase", required=True, choices=sorted(PHASES))
    common.add_argument("--trust", required=True, choices=sorted(TRUST))
    common.add_argument("--clock-domain", required=True, choices=sorted(CLOCK_DOMAINS), dest="clock_domain")
    common.add_argument("--run-id")
    common.add_argument("--attempt", type=int)
    common.add_argument("--occurrence", type=int)
    common.add_argument("--source-sha")
    common.add_argument("--image-fingerprint")
    common.add_argument("--cache-sha256")
    common.add_argument("--cache-generation", type=int)
    common.add_argument("--cache-state", choices=sorted(CACHE_STATES))
    command = argparse.ArgumentParser(description=__doc__)
    sub = command.add_subparsers(dest="command", required=True)
    sub.add_parser("start", parents=[common])
    end = sub.add_parser("end", parents=[common])
    end.add_argument("--outcome", required=True, choices=sorted(OUTCOMES))
    for name in ("bytes", "count", "test-count", "passed-test-count", "ignored-test-count"):
        end.add_argument(f"--{name}", type=int, dest=name.replace("-", "_"))
    validate = sub.add_parser("validate")
    validate.add_argument("--path", required=True)
    validate.add_argument("--require-phase", action="append", choices=sorted(PHASES), default=[])
    validate.add_argument("--require-supervisor-phase", action="append", choices=sorted(PHASES), default=[])
    validate.add_argument("--require-workload-phase", action="append", choices=sorted(PHASES), default=[])
    validate.add_argument("--require-trust", choices=sorted(TRUST))
    for option in (validate,):
        option.add_argument("--expected-run-id")
        option.add_argument("--expected-attempt", type=int)
        option.add_argument("--expected-source-sha")
        option.add_argument("--expected-image-fingerprint")
    project = sub.add_parser("project")
    project.add_argument("--path", required=True)
    project.add_argument("--output", required=True)
    project.add_argument("--require-phase", action="append", choices=sorted(PHASES), default=[])
    project.add_argument("--require-supervisor-phase", action="append", choices=sorted(PHASES), default=[])
    project.add_argument("--require-workload-phase", action="append", choices=sorted(PHASES), default=[])
    project.add_argument("--require-trust", choices=sorted(TRUST))
    project.add_argument("--expected-run-id")
    project.add_argument("--expected-attempt", type=int)
    project.add_argument("--expected-source-sha")
    project.add_argument("--expected-image-fingerprint")
    projection_parser = sub.add_parser("validate-projection")
    projection_parser.add_argument("--path", required=True)
    projection_parser.add_argument("--require-phase", action="append", choices=sorted(PHASES), default=[])
    projection_parser.add_argument("--require-supervisor-phase", action="append", choices=sorted(PHASES), default=[])
    projection_parser.add_argument("--require-workload-phase", action="append", choices=sorted(PHASES), default=[])
    projection_parser.add_argument("--expected-run-id")
    projection_parser.add_argument("--expected-attempt", type=int)
    projection_parser.add_argument("--expected-source-sha")
    projection_parser.add_argument("--expected-image-fingerprint")
    diagnose = sub.add_parser("diagnose")
    diagnose.add_argument("--path", required=True)
    diagnose.add_argument("--failed-stage", required=True, choices=sorted(TIMING_DIAGNOSTIC_STAGES))
    diagnose.add_argument("--require-phase", action="append", choices=sorted(PHASES), default=[])
    diagnose.add_argument("--require-trust", choices=sorted(TRUST))
    diagnose.add_argument("--expected-run-id")
    diagnose.add_argument("--expected-attempt", type=int)
    diagnose.add_argument("--expected-source-sha")
    diagnose.add_argument("--expected-image-fingerprint")
    markers = sub.add_parser("import-markers")
    markers.add_argument("--input", required=True)
    markers.add_argument("--output", required=True)
    markers.add_argument("--source-sha", required=True)
    markers.add_argument("--run-id")
    markers.add_argument("--attempt", type=int)
    markers.add_argument("--image-fingerprint")
    markers.add_argument("--clock-domain", choices=sorted(CLOCK_DOMAINS), default="workload-libkrun")
    return command


def run(args: argparse.Namespace) -> int:
    if args.command == "diagnose":
        summary = timing_diagnostic(
            Path(args.path),
            stage=args.failed_stage,
            required=set(args.require_phase),
            required_trust=args.require_trust,
            expected_run_id=args.expected_run_id,
            expected_attempt=args.expected_attempt,
            expected_source_sha=args.expected_source_sha,
            expected_image_fingerprint=args.expected_image_fingerprint,
        )
        print("HEPH_GCP_DIAGNOSTICS " + " ".join(f"{key}={value}" for key, value in summary.items()))
        return 0
    if args.command == "validate":
        path = path_for(args.path)
        validate_pairs(
            read_records(path), required=set(args.require_phase), required_trust=args.require_trust,
            expected_run_id=args.expected_run_id, expected_attempt=args.expected_attempt,
            expected_source_sha=args.expected_source_sha, expected_image_fingerprint=args.expected_image_fingerprint,
            required_supervisor=set(args.require_supervisor_phase), required_workload=set(args.require_workload_phase),
        )
        print("gcp phase timing: valid")
        return 0
    if args.command == "project":
        path = path_for(args.path)
        output = path_for(args.output)
        result = projection(
            validate_pairs(
                read_records(path), required=set(args.require_phase), required_trust=args.require_trust,
                expected_run_id=args.expected_run_id, expected_attempt=args.expected_attempt,
                expected_source_sha=args.expected_source_sha, expected_image_fingerprint=args.expected_image_fingerprint,
                required_supervisor=set(args.require_supervisor_phase), required_workload=set(args.require_workload_phase),
            )
        )
        encoded = json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n"
        if len(encoded.encode("utf-8")) > MAX_RECORD_BYTES * 4:
            fail("timing projection exceeds the size bound")
        output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        if any(parent.is_symlink() for parent in output.parents):
            fail("timing output parents must not be symlinks")
        descriptor = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
                descriptor = -1
                handle.write(encoded)
        finally:
            if descriptor >= 0:
                os.close(descriptor)
        return 0
    if args.command == "validate-projection":
        path = path_for(args.path)
        try:
            if path.stat().st_size > MAX_RECORD_BYTES * 4:
                fail("timing projection exceeds the size bound")
        except OSError:
            fail("timing projection cannot be read")
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, ValueError):
            fail("timing projection cannot be read")
        validate_projection(
            value,
            required=set(args.require_phase),
            expected_run_id=args.expected_run_id,
            expected_attempt=args.expected_attempt,
            expected_source_sha=args.expected_source_sha,
            expected_image_fingerprint=args.expected_image_fingerprint,
            required_supervisor=set(args.require_supervisor_phase),
            required_workload=set(args.require_workload_phase),
        )
        print("gcp phase timing: projection valid")
        return 0
    if args.command == "import-markers":
        return import_markers(args)
    path = path_for(args.path)
    identity_fields = identity(args)
    now = time.monotonic_ns()
    value: dict[str, Any] = {
        "schema": SCHEMA,
        "record": args.command,
        "phase": args.phase,
        "trust": args.trust,
        "clock_domain": args.clock_domain,
        "mono_ns": now,
        **identity_fields,
    }
    records = read_records(path)
    key = (args.phase, args.trust, args.clock_domain, identity_fields["occurrence"])
    open_records = {
        (record["phase"], record["trust"], record["clock_domain"], record["occurrence"]): record
        for record in records
        if record["record"] == "start"
    }
    if args.command == "start":
        if key in open_records:
            fail("duplicate phase start")
    else:
        start = open_records.get(key)
        if start is None:
            fail("phase end has no matching start")
        if any(start.get(name) != value.get(name) for name in IDENTITY_KEYS):
            fail("phase provenance changed between start and end")
        value["outcome"] = args.outcome
        for source, target in (
            ("bytes", "bytes"),
            ("count", "count"),
            ("test_count", "test_count"),
            ("passed_test_count", "passed_test_count"),
            ("ignored_test_count", "ignored_test_count"),
        ):
            item = getattr(args, source, None)
            if item is not None:
                value[target] = item
        if args.cache_state is not None:
            value["cache_state"] = args.cache_state
    validate_record(value, record=args.command)
    write_record(path, value)
    return 0


def main() -> int:
    args: argparse.Namespace | None = None
    try:
        args = parser().parse_args()
        return run(args)
    except (TimingError, OSError, ValueError) as exc:
        stage = {
            "start": "timing-helper-start",
            "end": "timing-helper-end",
        }.get(args.command if args is not None else "", "timing-helper-validation")
        print(
            "HEPH_GCP_COOKING event=timing-helper-error operation=cooking-workload "
            f"phase=cooking stage={stage} reason_class={shell_timing_error_class(exc)} status=failed exit_code=2",
            file=sys.stderr,
        )
        print(f"gcp phase timing: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
