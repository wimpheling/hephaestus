#!/usr/bin/env python3
"""Regression tests for the production project-build timing contract."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).parent
HELPER_PATH = ROOT / "gcp_phase_timing.py"
COLLECTOR_PATH = ROOT / "collect-cooking-diagnostics.py"
GOLDEN_PATH = ROOT.parent / "crates/hephaestus-app/tests/golden.rs"
SOURCE_SHA = "a" * 40
IMAGE_FINGERPRINT = "b" * 32


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PHASE_TIMING = load_module("gcp_phase_timing_production_build_test", HELPER_PATH)
COLLECTOR = load_module("cooking_diagnostics_production_build_test", COLLECTOR_PATH)


class ProductionProjectBuildTimingTests(unittest.TestCase):
    def run_helper(self, *arguments: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(HELPER_PATH), *arguments],
            check=check,
            capture_output=True,
            text=True,
        )

    def test_real_golden_producer_imports_and_strict_projection_allows_optional_host_build(self) -> None:
        """The Rust producer's marker satisfies the strict production profile."""

        producer = GOLDEN_PATH.read_text(encoding="utf-8")
        self.assertIn(
            'WorkloadPhaseTimer::start("production-project-build", workload_phase_timing)',
            producer,
        )
        self.assertIn("production_project_build_timer.finish(result.is_ok())", producer)
        self.assertIn(
            '"HEPH_GCP_COOKING event={WORKLOAD_PHASE_TIMING_EVENT} phase={} status={status} duration_ms={duration_ms}"',
            producer,
        )

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            markers = root / "golden-stderr.log"
            markers.write_text(
                "HEPH_GCP_COOKING event=phase-timing phase=production-project-build "
                "status=passed duration_ms=37\n",
                encoding="utf-8",
            )
            timing = root / "timing.jsonl"
            self.run_helper(
                "import-markers",
                "--input",
                str(markers),
                "--output",
                str(timing),
                "--source-sha",
                SOURCE_SHA,
                "--run-id",
                "34700000000",
                "--attempt",
                "1",
                "--image-fingerprint",
                IMAGE_FINGERPRINT,
            )

            projection = root / "production-only.json"
            self.run_helper(
                "project",
                "--path",
                str(timing),
                "--output",
                str(projection),
                "--require-workload-phase",
                "production-project-build",
                "--expected-run-id",
                "34700000000",
                "--expected-attempt",
                "1",
                "--expected-source-sha",
                SOURCE_SHA,
                "--expected-image-fingerprint",
                IMAGE_FINGERPRINT,
            )
            phases = json.loads(projection.read_text(encoding="utf-8"))["phases"]
            self.assertEqual([item["phase"] for item in phases], ["production-project-build"])
            self.assertEqual(phases[0]["clock_domain"], "workload-libkrun")
            self.assertEqual(phases[0]["measurement"], "informational")

            # The existing host compile is optional: adding it must remain
            # valid, while a host-only record must not satisfy production.
            host_args = [
                "--path",
                str(timing),
                "--phase",
                "project-build",
                "--trust",
                "workload",
                "--clock-domain",
                "workload",
                "--run-id",
                "34700000000",
                "--attempt",
                "1",
                "--source-sha",
                SOURCE_SHA,
                "--image-fingerprint",
                IMAGE_FINGERPRINT,
                "--occurrence",
                "1",
            ]
            self.run_helper("start", *host_args)
            self.run_helper("end", *host_args, "--outcome", "passed")
            with_host = root / "with-host.json"
            self.run_helper(
                "project",
                "--path",
                str(timing),
                "--output",
                str(with_host),
                "--require-workload-phase",
                "production-project-build",
            )
            self.assertEqual(len(json.loads(with_host.read_text(encoding="utf-8"))["phases"]), 2)

            host_only = root / "host-only.jsonl"
            host_only_args = list(host_args)
            host_only_args[host_only_args.index(str(timing))] = str(host_only)
            self.run_helper("start", *host_only_args)
            self.run_helper("end", *host_only_args, "--outcome", "passed")
            rejected = self.run_helper(
                "validate",
                "--path",
                str(host_only),
                "--require-workload-phase",
                "production-project-build",
                check=False,
            )
            self.assertNotEqual(rejected.returncode, 0)

    def test_helper_and_collector_share_phase_vocabulary_and_safe_projection_rejects_unknown(self) -> None:
        self.assertEqual(tuple(PHASE_TIMING.PHASE_ORDER), tuple(COLLECTOR.PHASE_TIMING_PHASE_ORDER))
        self.assertIn("production-project-build", PHASE_TIMING.WORKLOAD_PHASE_DOMAINS)
        self.assertEqual(
            PHASE_TIMING.WORKLOAD_PHASE_DOMAINS["production-project-build"],
            {"workload-libkrun"},
        )

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            projection = root / "projection.json"
            projection.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "phases": [
                            {
                                "phase": "production-project-build",
                                "trust": "workload",
                                "measurement": "informational",
                                "clock_domain": "workload-libkrun",
                                "duration_ms": 37,
                                "outcome": "passed",
                                "run_id": "34700000000",
                                "attempt": 1,
                                "occurrence": 1,
                                "source_sha": SOURCE_SHA,
                            }
                        ],
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            safe = root / "safe-phase-timing.json"
            COLLECTOR._project_phase_timing(projection, safe)
            self.assertEqual(
                json.loads(safe.read_text(encoding="utf-8"))["phases"][0]["phase"],
                "production-project-build",
            )

            unsafe = root / "unsafe-phase-timing.json"
            value = json.loads(projection.read_text(encoding="utf-8"))
            value["phases"][0]["phase"] = "private-build-detail"
            unsafe.write_text(json.dumps(value) + "\n", encoding="utf-8")
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR._project_phase_timing(unsafe, root / "rejected.json")


if __name__ == "__main__":
    unittest.main()
