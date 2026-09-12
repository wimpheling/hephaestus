#!/usr/bin/env python3
"""Focused contract tests for the safe GCP Cooking phase timing helper."""

from __future__ import annotations

import json
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("gcp_phase_timing.py")
SOURCE = "a" * 40
IMAGE = "b" * 32
CACHE = "c" * 64

HELPER_SPEC = importlib.util.spec_from_file_location("gcp_phase_timing_under_test", SCRIPT)
assert HELPER_SPEC is not None and HELPER_SPEC.loader is not None
PHASE_TIMING = importlib.util.module_from_spec(HELPER_SPEC)
HELPER_SPEC.loader.exec_module(PHASE_TIMING)


class CountingReader:
    def __init__(self, handle, limit: int, on_close) -> None:
        self._handle = handle
        self._limit = limit
        self._on_close = on_close
        self.reads = 0

    def _count(self) -> None:
        self.reads += 1
        if self.reads > self._limit:
            raise AssertionError("diagnostic fallback read beyond its bounded prefix")

    def readline(self, *args):
        self._count()
        return self._handle.readline(*args)

    def __iter__(self):
        while True:
            line = self.readline()
            if not line:
                return
            yield line

    def __enter__(self):
        self._handle.__enter__()
        return self

    def __exit__(self, *args):
        self._on_close(self.reads)
        return self._handle.__exit__(*args)


class CountingPath:
    def __init__(self, path: Path, limit: int) -> None:
        self._path = path
        self._limit = limit
        self.reads = 0

    def exists(self):
        return self._path.exists()

    def is_symlink(self):
        return self._path.is_symlink()

    def is_file(self):
        return self._path.is_file()

    @property
    def parents(self):
        return self._path.parents

    def open(self, *args, **kwargs):
        return CountingReader(
            self._path.open(*args, **kwargs), self._limit, lambda reads: setattr(self, "reads", reads)
        )


class PhaseTimingTests(unittest.TestCase):
    def run_cli(self, *arguments: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), *arguments],
            check=check,
            capture_output=True,
            text=True,
        )

    def common(self, path: Path, phase: str = "cache-download") -> list[str]:
        return [
            "--path",
            str(path),
            "--phase",
            phase,
            "--trust",
            "supervisor",
            "--clock-domain",
            "guest-runtime",
            "--run-id",
            "123",
            "--attempt",
            "1",
            "--source-sha",
            SOURCE,
            "--image-fingerprint",
            IMAGE,
            "--cache-sha256",
            CACHE,
            "--cache-generation",
            "17",
            "--cache-state",
            "miss",
        ]

    def test_start_end_and_projection_marks_workload_informational(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "timing.jsonl"
            self.run_cli("start", *self.common(path))
            self.run_cli("end", *self.common(path), "--outcome", "passed", "--bytes", "99", "--count", "4")
            output = path.with_name("projection.json")
            self.run_cli("project", "--path", str(path), "--output", str(output), "--require-phase", "cache-download")
            value = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(value["schema"], 1)
            phase = value["phases"][0]
            self.assertEqual(phase["measurement"], "trusted")
            self.assertGreaterEqual(phase["duration_ms"], 0)
            self.assertEqual(phase["bytes"], 99)
            self.assertNotIn("mono_ns", phase)

            hit = path.with_name("cache-hit.jsonl")
            hit_args = self.common(hit)
            hit_args[hit_args.index("miss")] = "hit"
            self.run_cli("start", *hit_args)
            self.run_cli("end", *hit_args, "--outcome", "passed", "--bytes", "101")
            self.run_cli("validate", "--path", str(hit), "--require-phase", "cache-download")
            hit_projection = hit.with_name("cache-hit-projection.json")
            self.run_cli("project", "--path", str(hit), "--output", str(hit_projection))
            self.assertEqual(json.loads(hit_projection.read_text())["phases"][0]["cache_state"], "hit")

            workload = path.with_name("workload.jsonl")
            workload_args = self.common(workload, "golden-tests")
            workload_args[workload_args.index("supervisor")] = "workload"
            workload_args[workload_args.index("guest-runtime")] = "workload-libkrun"
            self.run_cli("start", *workload_args)
            self.run_cli("end", *workload_args, "--outcome", "passed")
            projected = workload.with_name("workload-projection.json")
            self.run_cli("project", "--path", str(workload), "--output", str(projected))
            self.assertEqual(json.loads(projected.read_text())["phases"][0]["measurement"], "informational")

    def test_missing_and_duplicate_phases_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "timing.jsonl"
            common = self.common(path)
            self.assertNotEqual(self.run_cli("end", *common, "--outcome", "passed", check=False).returncode, 0)
            self.run_cli("start", *common)
            self.assertNotEqual(self.run_cli("start", *common, check=False).returncode, 0)
            self.assertNotEqual(self.run_cli("validate", "--path", str(path), check=False).returncode, 0)
            self.run_cli("end", *common, "--outcome", "timed-out")
            self.assertNotEqual(
                self.run_cli("validate", "--path", str(path), "--require-phase", "cache-extract", check=False).returncode,
                0,
            )

    def test_validate_pairs_rejects_unclosed_unmatched_and_reversed_intervals(self) -> None:
        common = {
            "schema": 1,
            "phase": "archive",
            "trust": "supervisor",
            "clock_domain": "guest-startup",
            "run_id": "123",
            "attempt": 1,
            "occurrence": 2,
            "source_sha": SOURCE,
        }
        start = {**common, "record": "start", "mono_ns": 10}
        end = {**common, "record": "end", "mono_ns": 20, "outcome": "passed"}

        with self.assertRaisesRegex(PHASE_TIMING.TimingError, "incomplete phase"):
            PHASE_TIMING.validate_pairs([start])
        with self.assertRaisesRegex(PHASE_TIMING.TimingError, "no matching start"):
            PHASE_TIMING.validate_pairs([end])
        with self.assertRaisesRegex(PHASE_TIMING.TimingError, "precedes phase start"):
            PHASE_TIMING.validate_pairs([start, {**end, "mono_ns": 9}])

        complete = PHASE_TIMING.validate_pairs(
            [start, end], required_supervisor={"archive"}, expected_run_id="123", expected_attempt=1
        )
        self.assertEqual(len(complete), 1)
        self.assertEqual(complete[0]["occurrence"], 2)
        self.assertEqual(complete[0]["clock_domain"], "guest-startup")
        with self.assertRaisesRegex(PHASE_TIMING.TimingError, "clock domain"):
            PHASE_TIMING.validate_pairs(
                [
                    {**start, "clock_domain": "controller"},
                    {**end, "clock_domain": "controller"},
                ],
                required_supervisor={"archive"},
            )
        with self.assertRaisesRegex(PHASE_TIMING.TimingError, "duplicate phase start"):
            PHASE_TIMING.validate_pairs([start, start, end])

    def test_validate_rejects_unhashable_phase_domain_and_occurrence_without_traceback(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            base = {
                "schema": 1,
                "record": "start",
                "phase": "archive",
                "trust": "supervisor",
                "clock_domain": "guest-startup",
                "mono_ns": 1,
                "run_id": "123",
                "attempt": 1,
                "occurrence": 1,
                "source_sha": SOURCE,
            }
            for field, value in (
                ("phase", ["archive", "PRIVATE_PAYLOAD"]),
                ("clock_domain", {"domain": "guest-startup"}),
                ("occurrence", [1]),
            ):
                with self.subTest(field=field):
                    path = root / f"bad-{field}.jsonl"
                    path.write_text(json.dumps({**base, field: value}) + "\n", encoding="utf-8")
                    result = self.run_cli("validate", "--path", str(path), check=False)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertNotIn("Traceback", result.stderr)
                    self.assertNotIn("PRIVATE_PAYLOAD", result.stdout + result.stderr)

    def test_shell_pairing_failures_keep_legacy_pair_reason(self) -> None:
        for message in (
            "phase end has no matching start",
            "phase end precedes phase start",
            "timing file contains an incomplete phase",
        ):
            with self.subTest(message=message):
                self.assertEqual(PHASE_TIMING.shell_timing_error_class(PHASE_TIMING.TimingError(message)), "pair")

    def test_pairing_diagnostics_distinguish_failure_classes(self) -> None:
        cases = {
            "timing file contains an incomplete phase": "unclosed-start",
            "phase end has no matching start": "unmatched-end",
            "phase end precedes phase start": "end-before-start",
        }
        for message, expected in cases.items():
            with self.subTest(message=message):
                self.assertEqual(PHASE_TIMING.timing_error_class(PHASE_TIMING.TimingError(message)), expected)

    def test_helper_failures_emit_fixed_safe_categories(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            invalid_identity = self.run_cli(
                "start", *self.common(root / "identity.jsonl"), "--source-sha", "bad", check=False
            )
            self.assertEqual(invalid_identity.returncode, 2)
            self.assertIn(
                "stage=timing-helper-start reason_class=identity status=failed exit_code=2",
                invalid_identity.stderr,
            )

            invalid_path = root / "directory"
            invalid_path.mkdir()
            path_result = self.run_cli("start", *self.common(invalid_path), check=False)
            self.assertEqual(path_result.returncode, 2)
            self.assertIn(
                "stage=timing-helper-start reason_class=path status=failed exit_code=2",
                path_result.stderr,
            )

            pair_path = root / "pair.jsonl"
            self.run_cli("start", *self.common(pair_path))
            pair_result = self.run_cli(
                "end", *self.common(pair_path), "--occurrence", "2", "--outcome", "failed", check=False
            )
            self.assertEqual(pair_result.returncode, 2)
            self.assertIn(
                "stage=timing-helper-end reason_class=pair status=failed exit_code=2",
                pair_result.stderr,
            )

    def test_cancellation_is_a_valid_terminal_outcome(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "timing.jsonl"
            common = self.common(path, "cleanup-verification")
            self.run_cli("start", *common)
            self.run_cli("end", *common, "--outcome", "cancelled")
            self.run_cli("validate", "--path", str(path))
            self.assertEqual(json.loads(path.read_text().splitlines()[1])["outcome"], "cancelled")

    def test_malformed_fields_secrets_and_oversized_records_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            malformed = root / "malformed.jsonl"
            malformed.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "record": "end",
                        "phase": "cache-download",
                        "trust": "supervisor",
                        "clock_domain": "guest-runtime",
                        "mono_ns": 1,
                        "run_id": "1",
                        "attempt": 1,
                        "source_sha": SOURCE,
                        "outcome": "passed",
                        "command": "Authorization: Bearer secret",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            malformed_result = self.run_cli("validate", "--path", str(malformed), check=False)
            self.assertNotEqual(malformed_result.returncode, 0)
            self.assertNotIn("secret", malformed_result.stderr)
            typed = root / "typed.jsonl"
            typed.write_text(
                json.dumps(
                    {
                        "schema": True,
                        "record": "end",
                        "phase": "cache-download",
                        "trust": "supervisor",
                        "clock_domain": "guest-runtime",
                        "mono_ns": 1,
                        "run_id": "1",
                        "attempt": 1,
                        "source_sha": SOURCE,
                        "cache_state": ["miss"],
                        "outcome": ["passed"],
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            typed_result = self.run_cli("validate", "--path", str(typed), check=False)
            self.assertNotEqual(typed_result.returncode, 0)
            self.assertNotIn("Traceback", typed_result.stderr)
            typed_value = json.loads(typed.read_text(encoding="utf-8"))
            typed_value["cache_state"] = "miss"
            typed.write_text(json.dumps(typed_value) + "\n", encoding="utf-8")
            typed_result = self.run_cli("validate", "--path", str(typed), check=False)
            self.assertNotEqual(typed_result.returncode, 0)
            self.assertNotIn("Traceback", typed_result.stderr)
            oversized = root / "oversized.jsonl"
            oversized.write_text("x" * 20_000 + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_cli("validate", "--path", str(oversized), check=False).returncode, 0)

    def test_diagnose_keeps_malformed_values_bounded_and_typed(self) -> None:
        """Malformed input yields a finite safe diagnostic without echoing it."""

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            malformed = root / "malformed-diagnostic.jsonl"
            malformed.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "record": "start",
                        "phase": ["project-build", "PRIVATE_PAYLOAD"],
                        "trust": "workload",
                        "clock_domain": "workload",
                        "mono_ns": 1,
                        "run_id": "123",
                        "attempt": 1,
                        "occurrence": 1,
                        "source_sha": SOURCE,
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_cli(
                "diagnose",
                "--path",
                str(malformed),
                "--failed-stage",
                "projection",
                "--require-phase",
                "project-build",
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(
                "HEPH_GCP_DIAGNOSTICS event=phase-timing status=unavailable "
                "failed_stage=projection reason_class=invalid failed_phase=none "
                "failed_clock_domain=none failed_occurrence=0 available_count=0 "
                "available_phases=none missing_count=1 missing_phases=project-build",
                result.stdout,
            )
            self.assertNotIn("PRIVATE_PAYLOAD", result.stdout + result.stderr)
            self.assertNotIn("Traceback", result.stderr)

    def test_diagnose_limits_fallback_scan_of_excessive_short_records(self) -> None:
        """A malformed prefix cannot make diagnosis scan an unbounded file."""

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "many-short-records.jsonl"
            source.write_text("{}\n" * (PHASE_TIMING.MAX_RECORDS * 4), encoding="utf-8")
            bounded = CountingPath(source, PHASE_TIMING.MAX_RECORDS + 2)
            summary = PHASE_TIMING.timing_diagnostic(
                bounded,
                stage="projection",
                required={"project-build"},
            )
            self.assertEqual(summary["status"], "unavailable")
            self.assertEqual(summary["reason_class"], "invalid")
            self.assertEqual(summary["available_phases"], "none")
            self.assertEqual(summary["missing_phases"], "project-build")
            self.assertLessEqual(bounded.reads, PHASE_TIMING.MAX_RECORDS + 2)

            oversized = root / "oversized-diagnostic.jsonl"
            oversized.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "record": "start",
                        "phase": "project-build",
                        "trust": "workload",
                        "clock_domain": "workload",
                        "mono_ns": 1,
                        "run_id": "123",
                        "attempt": 1,
                        "occurrence": 1,
                        "source_sha": SOURCE,
                        "payload": "PRIVATE_PAYLOAD" * 2_000,
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_cli(
                "diagnose",
                "--path",
                str(oversized),
                "--failed-stage",
                "projection",
                "--require-phase",
                "project-build",
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertLess(len(result.stdout), 1024)
            self.assertNotIn("PRIVATE_PAYLOAD", result.stdout + result.stderr)

    def test_invalid_timestamp_identity_and_counter_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "invalid.jsonl"
            path.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "record": "start",
                        "phase": "cache-download",
                        "trust": "supervisor",
                        "clock_domain": "guest-runtime",
                        "mono_ns": -1,
                        "run_id": "1",
                        "attempt": 1,
                        "source_sha": "bad",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            self.assertNotEqual(self.run_cli("validate", "--path", str(path), check=False).returncode, 0)

            cache = root = Path(directory) / "counter.jsonl"
            args = self.common(cache)
            args[args.index("17")] = str(10**20)
            self.assertNotEqual(self.run_cli("start", *args, check=False).returncode, 0)

    def test_end_provenance_mismatch_and_negative_metrics_fail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "timing.jsonl"
            common = self.common(path)
            self.run_cli("start", *common)
            changed = list(common)
            changed[changed.index(SOURCE)] = "d" * 40
            self.assertNotEqual(self.run_cli("end", *changed, "--outcome", "passed", check=False).returncode, 0)
            self.assertNotEqual(
                self.run_cli("end", *common, "--outcome", "passed", "--bytes", "-1", check=False).returncode,
                0,
            )

    def test_validator_rejects_duplicate_and_out_of_order_records(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            duplicate = root / "duplicate.jsonl"
            common = self.common(duplicate)
            self.run_cli("start", *common)
            self.run_cli("end", *common, "--outcome", "passed")
            first_start = duplicate.read_text(encoding="utf-8").splitlines()[0]
            with duplicate.open("a", encoding="utf-8") as handle:
                handle.write(first_start + "\n")
            self.assertNotEqual(self.run_cli("validate", "--path", str(duplicate), check=False).returncode, 0)

            out_of_order = root / "out-of-order.jsonl"
            base = {
                "schema": 1,
                "record": "start",
                "trust": "supervisor",
                "clock_domain": "controller",
                "run_id": "1",
                "attempt": 1,
                "source_sha": SOURCE,
            }
            records = [
                {**base, "phase": "cleanup-verification", "mono_ns": 1},
                {**base, "phase": "preflight-quota", "mono_ns": 2},
            ]
            out_of_order.write_text("".join(json.dumps(item) + "\n" for item in records), encoding="utf-8")
            self.assertNotEqual(self.run_cli("validate", "--path", str(out_of_order), check=False).returncode, 0)

    def test_nested_repeated_and_concurrent_workload_phases_validate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nested.jsonl"
            outer = self.common(path, "golden-tests")
            outer[outer.index("supervisor")] = "workload"
            outer[outer.index("guest-runtime")] = "workload-libkrun"
            self.run_cli("start", *outer)
            nested = list(outer)
            nested[nested.index("golden-tests")] = "oci-builder"
            nested.extend(("--occurrence", "1"))
            self.run_cli("start", *nested)
            self.run_cli("end", *nested, "--outcome", "passed")
            repeated = list(outer)
            repeated[repeated.index("golden-tests")] = "oci-builder"
            repeated.extend(("--occurrence", "2"))
            self.run_cli("start", *repeated)
            self.run_cli("end", *repeated, "--outcome", "passed")
            self.run_cli("end", *outer, "--outcome", "passed")

            edge = self.common(path, "gateway-edge-ready")
            edge[edge.index("supervisor")] = "workload"
            edge[edge.index("guest-runtime")] = "workload-gateway"
            self.run_cli("start", *edge)
            self.run_cli("end", *edge, "--outcome", "passed")
            self.run_cli("validate", "--path", str(path), "--require-trust", "workload")

    def test_marker_import_is_strict_and_workload_origin_is_enforced(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            markers = root / "markers.log"
            markers.write_text(
                "ordinary output\n"
                "HEPH_GCP_COOKING event=phase-timing phase=oci-builder status=passed duration_ms=12\n"
                "HEPH_GCP_COOKING event=phase-timing phase=oci-builder status=failed duration_ms=7\n",
                encoding="utf-8",
            )
            output = root / "timing.jsonl"
            self.run_cli(
                "import-markers",
                "--input",
                str(markers),
                "--output",
                str(output),
                "--source-sha",
                SOURCE,
                "--run-id",
                "123",
                "--attempt",
                "1",
            )
            self.run_cli("validate", "--path", str(output), "--require-trust", "workload")
            records = [json.loads(line) for line in output.read_text().splitlines()]
            self.assertEqual({record["occurrence"] for record in records}, {1, 2})

            malformed = root / "malformed-markers.log"
            malformed.write_text(
                "HEPH_GCP_COOKING event=phase-timing phase=oci-builder status=passed duration_ms=secret\n",
                encoding="utf-8",
            )
            malformed_result = self.run_cli(
                "import-markers",
                "--input",
                str(malformed),
                "--output",
                str(root / "malformed.jsonl"),
                "--source-sha",
                SOURCE,
                check=False,
            )
            self.assertNotEqual(malformed_result.returncode, 0)
            self.assertNotIn("secret", malformed_result.stderr)

            supervisor_output = root / "supervisor.jsonl"
            self.run_cli("start", *self.common(supervisor_output))
            self.assertNotEqual(
                self.run_cli(
                    "import-markers",
                    "--input",
                    str(markers),
                    "--output",
                    str(supervisor_output),
                    "--source-sha",
                    SOURCE,
                    check=False,
                ).returncode,
                0,
            )
            self.assertNotEqual(
                self.run_cli(
                    "validate",
                    "--path",
                    str(supervisor_output),
                    "--require-trust",
                    "workload",
                    check=False,
                ).returncode,
                0,
            )

    def test_success_projection_combines_five_rust_markers_with_shell_phases(self) -> None:
        """Rust workload markers and shell timers form one valid projection."""

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            markers = root / "rust-stderr.log"
            markers.write_text(
                "\n".join(
                    f"HEPH_GCP_COOKING event=phase-timing phase={phase} status=passed duration_ms={duration}"
                    for phase, duration in (
                        ("runtime-guest-build", 11),
                        ("runtime-worker-build", 13),
                        ("oci-builder", 17),
                        ("oci-verifier", 19),
                        ("golden-tests", 23),
                    )
                )
                + "\n",
                encoding="utf-8",
            )
            timing = root / "timing.jsonl"
            self.run_cli(
                "import-markers",
                "--input",
                str(markers),
                "--output",
                str(timing),
                "--source-sha",
                SOURCE,
                "--run-id",
                "98765",
                "--attempt",
                "2",
                "--image-fingerprint",
                IMAGE,
            )

            for phase, clock_domain in (
                ("dependency-setup", "workload"),
                ("project-build", "workload"),
                ("gateway-edge-ready", "workload-gateway"),
            ):
                shell_args = self.common(timing, phase)
                shell_args[shell_args.index("supervisor")] = "workload"
                shell_args[shell_args.index("guest-runtime")] = clock_domain
                shell_args[shell_args.index("123")] = "98765"
                shell_args[shell_args.index("1")] = "2"
                shell_args[shell_args.index(IMAGE)] = IMAGE
                self.run_cli("start", *shell_args)
                self.run_cli("end", *shell_args, "--outcome", "passed")

            required = [
                "dependency-setup",
                "project-build",
                "gateway-edge-ready",
                "runtime-guest-build",
                "runtime-worker-build",
                "oci-builder",
                "oci-verifier",
                "golden-tests",
            ]
            validate_args = ["validate", "--path", str(timing), "--require-trust", "workload"]
            for phase in required:
                validate_args.extend(("--require-workload-phase", phase))
            self.run_cli(*validate_args, "--expected-run-id", "98765", "--expected-attempt", "2", "--expected-source-sha", SOURCE, "--expected-image-fingerprint", IMAGE)

            projection = root / "projection.json"
            project_args = ["project", "--path", str(timing), "--output", str(projection)]
            for phase in required:
                project_args.extend(("--require-workload-phase", phase))
            self.run_cli(
                *project_args,
                "--expected-run-id",
                "98765",
                "--expected-attempt",
                "2",
                "--expected-source-sha",
                SOURCE,
                "--expected-image-fingerprint",
                IMAGE,
            )
            value = json.loads(projection.read_text(encoding="utf-8"))
            self.assertEqual(len(value["phases"]), len(required))
            self.assertEqual({phase["phase"] for phase in value["phases"]}, set(required))
            self.assertTrue(all(phase["measurement"] == "informational" for phase in value["phases"]))
            self.assertTrue(all(phase["run_id"] == "98765" and phase["attempt"] == 2 for phase in value["phases"]))

    def test_symlinked_timing_paths_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "target"
            target.mkdir()
            link = root / "link"
            os.symlink(target, link)
            result = self.run_cli(
                "start",
                *self.common(link / "timing.jsonl"),
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)

    def test_complete_cooking_profile_accepts_nested_domains_and_rejects_missing_critical_phase(self) -> None:
        required = [
            "dependency-setup",
            "project-build",
            "browser-setup",
            "runtime-guest-build",
            "runtime-worker-build",
            "oci-image-materialization",
            "gateway-edge-ready",
            "gateway-services-ready",
            "gateway-readiness",
            "oci-builder",
            "oci-verifier",
            "golden-tests",
            "database-tests",
            "browser-initial",
            "browser-post-operation",
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "complete.jsonl"
            records: list[dict[str, object]] = []
            clock = {"workload-libkrun": 1, "workload-gateway": 1}
            for index, phase in enumerate(required):
                domain = "workload-gateway" if phase in {"gateway-edge-ready", "gateway-services-ready"} else "workload-libkrun"
                start = clock[domain]
                common = {
                    "schema": 1,
                    "trust": "workload",
                    "clock_domain": domain,
                    "run_id": "1",
                    "attempt": 1,
                    "occurrence": 1,
                    "source_sha": SOURCE,
                    "phase": phase,
                }
                records.append({**common, "record": "start", "mono_ns": start})
                records.append(
                    {
                        **common,
                        "record": "end",
                        "mono_ns": start + index + 1,
                        "outcome": "cancelled" if phase == "browser-post-operation" else "passed",
                    }
                )
                clock[domain] = start + index + 2
            path.write_text("".join(json.dumps(item) + "\n" for item in records), encoding="utf-8")
            args = ["--path", str(path), "--require-trust", "workload"]
            for phase in required:
                args.extend(("--require-phase", phase))
            self.run_cli("validate", *args)
            path.write_text(
                "".join(json.dumps(item) + "\n" for item in records if item["phase"] != "browser-post-operation"),
                encoding="utf-8",
            )
            self.assertNotEqual(self.run_cli("validate", *args, check=False).returncode, 0)

    def test_final_projection_enforces_global_identity_and_trust_domains(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            raw = root / "full.jsonl"
            records: list[dict[str, object]] = []
            for index, (phase, trust, domain) in enumerate(
                (
                    ("archive", "supervisor", "guest-startup"),
                    ("evidence-scan", "supervisor", "guest-startup"),
                    ("upload", "supervisor", "guest-startup"),
                    ("project-build", "workload", "workload"),
                )
            ):
                common = {
                    "schema": 1,
                    "phase": phase,
                    "trust": trust,
                    "clock_domain": domain,
                    "run_id": "1",
                    "attempt": 1,
                    "occurrence": 1,
                    "source_sha": SOURCE,
                }
                records.extend(
                    (
                        {**common, "record": "start", "mono_ns": index * 10},
                        {**common, "record": "end", "mono_ns": index * 10 + 1, "outcome": "passed"},
                    )
                )
            raw.write_text("".join(json.dumps(item) + "\n" for item in records), encoding="utf-8")
            projection = root / "projection.json"
            self.run_cli("project", "--path", str(raw), "--output", str(projection), "--expected-run-id", "1", "--expected-attempt", "1", "--expected-source-sha", SOURCE)
            self.run_cli(
                "validate-projection",
                "--path",
                str(projection),
                "--expected-run-id",
                "1",
                "--expected-attempt",
                "1",
                "--expected-source-sha",
                SOURCE,
                "--require-supervisor-phase",
                "archive",
                "--require-supervisor-phase",
                "evidence-scan",
                "--require-supervisor-phase",
                "upload",
            )
            changed = [dict(item) for item in records]
            changed[-1]["source_sha"] = "d" * 40
            raw.write_text("".join(json.dumps(item) + "\n" for item in changed), encoding="utf-8")
            self.assertNotEqual(
                self.run_cli("validate", "--path", str(raw), check=False).returncode,
                0,
            )

    def test_final_projection_rebuild_includes_phases_closed_after_archive_snapshot(self) -> None:
        """Model the collector snapshot followed by terminal phase closure."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workload = root / "workload.jsonl"
            supervisor = root / "supervisor.jsonl"
            combined = root / "combined.jsonl"
            final_combined = root / "final-combined.jsonl"
            common = {
                "schema": 1,
                "trust": "workload",
                "clock_domain": "workload",
                "run_id": "1",
                "attempt": 1,
                "occurrence": 1,
                "source_sha": SOURCE,
            }
            workload.write_text(
                json.dumps({**common, "record": "start", "phase": "project-build", "mono_ns": 1})
                + "\n"
                + json.dumps({**common, "record": "end", "phase": "project-build", "mono_ns": 2, "outcome": "passed"})
                + "\n",
                encoding="utf-8",
            )
            supervisor_common = {
                **{key: value for key, value in common.items() if key != "clock_domain"},
                "trust": "supervisor",
                "clock_domain": "guest-startup",
            }
            supervisor.write_text(
                json.dumps({**supervisor_common, "record": "start", "phase": "cooking-supervisor", "mono_ns": 1})
                + "\n"
                + json.dumps({**supervisor_common, "record": "end", "phase": "cooking-supervisor", "mono_ns": 2, "outcome": "passed"})
                + "\n",
                encoding="utf-8",
            )
            combined.write_text(workload.read_text() + supervisor.read_text(), encoding="utf-8")
            initial = root / "initial.json"
            self.run_cli("project", "--path", str(combined), "--output", str(initial), "--require-workload-phase", "project-build")

            terminal = []
            for index, phase in enumerate(("archive", "evidence-scan", "upload"), start=3):
                terminal.extend(
                    (
                        {**supervisor_common, "record": "start", "phase": phase, "mono_ns": index * 10},
                        {**supervisor_common, "record": "end", "phase": phase, "mono_ns": index * 10 + 1, "outcome": "passed"},
                    )
                )
            with supervisor.open("a", encoding="utf-8") as handle:
                handle.write("".join(json.dumps(item) + "\n" for item in terminal))
            final_combined.write_text(workload.read_text() + supervisor.read_text(), encoding="utf-8")
            final = root / "final.json"
            self.run_cli(
                "project",
                "--path",
                str(final_combined),
                "--output",
                str(final),
                "--expected-run-id",
                "1",
                "--expected-attempt",
                "1",
                "--expected-source-sha",
                SOURCE,
                "--require-workload-phase",
                "project-build",
                "--require-supervisor-phase",
                "archive",
                "--require-supervisor-phase",
                "evidence-scan",
                "--require-supervisor-phase",
                "upload",
            )
            self.run_cli(
                "validate-projection",
                "--path",
                str(final),
                "--require-supervisor-phase",
                "archive",
                "--require-supervisor-phase",
                "evidence-scan",
                "--require-supervisor-phase",
                "upload",
            )


if __name__ == "__main__":
    unittest.main()
