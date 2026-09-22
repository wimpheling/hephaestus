"""Focused contracts for bounded first-failure and partial evidence reporting."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import tarfile
import textwrap
import unittest


ROOT = Path(__file__).parent


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


COLLECTOR = load("failure_collector", ROOT / "collect-cooking-diagnostics.py")
SUMMARY = load("failure_summary", ROOT / "summarize-cooking-diagnostics.py")
SCANNER = load("failure_scanner", ROOT / "check-browser-evidence.py")
TIMING = ROOT / "gcp_phase_timing.py"


class FailureReportingTests(unittest.TestCase):
    def test_first_failure_and_explicit_logs_are_scanned_and_projected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-first-failure-") as raw:
            root = Path(raw)
            sources = root / "sources"
            sources.mkdir()
            (sources / "first").write_text(
                json.dumps({
                    "schema": 1,
                    "phase": "browser-setup",
                    "command_id": "browser-setup",
                    "exit_code": 78,
                    "diagnostic_source": "setup-log",
                    "diagnostic_error": "setup-failed",
                }) + "\n",
                encoding="utf-8",
            )
            for label in ("setup-log", "image-build-log", "npm-log", "playwright-log", "browser-container-log"):
                (sources / label).write_text(
                    f"HEPH_GCP_DIAGNOSTIC phase=browser-initial event={label} status=failed exit_code=1\n"
                    f"HEPH_GCP_DIAGNOSTIC phase=browser-recovery event={label} status=downstream\n",
                    encoding="utf-8",
                )
            output = root / "bundle"
            status = COLLECTOR.collect(
                output,
                [
                    f"first-failure={sources / 'first'}",
                    *[f"{label}={sources / label}" for label in ("setup-log", "image-build-log", "npm-log", "playwright-log", "browser-container-log")],
                ],
                None,
                None,
                None,
            )
            self.assertEqual(status, 0)
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["credentialScan"], "passed")
            self.assertEqual({item["label"] for item in manifest["sources"]}, {
                "first-failure", "setup-log", "image-build-log", "npm-log", "playwright-log", "browser-container-log",
            })
            triage = SUMMARY.summarize(output)
            self.assertEqual(triage["firstFailure"]["command_id"], "browser-setup")
            self.assertEqual(triage["firstFailure"]["exit_code"], 78)
            retained_setup = (output / "sources" / "setup-log").read_text(encoding="utf-8")
            self.assertIn("phase=browser-initial", retained_setup)
            self.assertIn("phase=browser-recovery", retained_setup)

    def test_startup_log_collection_contract_covers_prerequisite_and_head_tail(self) -> None:
        source = (ROOT / "gcp-kvm-startup.sh").read_text(encoding="utf-8")
        self.assertIn('browser-prerequisite.*/browser.log', source)
        self.assertIn('browser-prerequisite.*/probe-*.log', source)
        self.assertIn('installed-ui-browser-image-load/podman.log', source)
        self.assertIn('head -c 4194304', source)
        self.assertIn('tail -c 4194304', source)

    def test_partial_projection_keeps_durations_but_strict_validation_fails(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-partial-timing-") as raw:
            root = Path(raw)
            records = root / "records.jsonl"
            projection = root / "projection.json"
            common = [
                "--path", str(records), "--phase", "browser-setup", "--trust", "workload",
                "--clock-domain", "workload", "--run-id", "12345", "--attempt", "1",
                "--source-sha", "a" * 40,
            ]
            for args in (("start", *common), ("end", *common, "--outcome", "failed")):
                self.assertEqual(subprocess.run([sys.executable, str(TIMING), *args], check=False).returncode, 0)
            result = subprocess.run(
                [sys.executable, str(TIMING), "project", "--allow-partial", "--path", str(records),
                 "--output", str(projection), "--expected-run-id", "12345", "--expected-attempt", "1",
                 "--expected-source-sha", "a" * 40, "--require-phase", "browser-setup", "--require-phase", "browser-initial"],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            value = json.loads(projection.read_text(encoding="utf-8"))
            self.assertEqual(value["completeness"], "partial")
            self.assertEqual(value["missing_phases"], ["browser-initial"])
            strict = subprocess.run(
                [sys.executable, str(TIMING), "validate-projection", "--path", str(projection),
                 "--expected-run-id", "12345", "--expected-attempt", "1", "--expected-source-sha", "a" * 40,
                 "--require-phase", "browser-initial"],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertNotEqual(strict.returncode, 0)

    def test_injected_setup_capture_archives_scans_and_triages_partial_evidence(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-injected-setup-e2e-") as raw:
            root = Path(raw)
            first_failure = root / "first-failure.json"
            setup_log = root / "setup.log"
            timing = root / "timing.jsonl"
            projection = root / "phase-timing.json"
            setup_log.write_text(
                "HEPH_GCP_DIAGNOSTIC event=setup phase=browser-setup status=failed exit_code=78\n",
                encoding="utf-8",
            )
            capture = r'''
source "$1"
failure_capture_disabled=false
first_failure_recorded=false
first_failure_diagnostic_source=setup-log
phase=browser-setup
revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
run_id=12345
run_attempt=1
timing_image_fingerprint=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
trusted_phase_timing_script="$PWD/scripts/gcp_phase_timing.py"
supervisor_phase_timing_path="$HEPH_GCP_TEST_TIMING_PATH"
supervisor_phase_timing_start browser-setup
set +e
bash -Eeuo pipefail -c 'exit 78'
status=$?
set -e
supervisor_phase_timing_end failed
for timing_phase in archive evidence-scan upload; do
  supervisor_phase_timing_start "$timing_phase"
  supervisor_phase_timing_end passed
done
test "$status" -eq 78
test -s "$first_failure_path"
'''
            env = {
                **dict(),
                "HEPH_GCP_STARTUP_LIBRARY": "1",
            }
            result = subprocess.run(
                ["bash", "-Eeuo", "pipefail", "-c", capture, "capture", str(ROOT / "gcp-kvm-startup.sh"), str(first_failure)],
                env={**env, **{"PATH": "/usr/bin:/bin", "HEPH_GCP_FIRST_FAILURE_PATH": str(first_failure), "HEPH_GCP_TEST_TIMING_PATH": str(root / "supervisor-timing.jsonl")}},
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            captured = json.loads(first_failure.read_text(encoding="utf-8"))
            self.assertEqual(captured["exit_code"], 78)
            self.assertEqual(captured["phase"], "browser-setup")
            self.assertEqual(captured["command_id"], "diagnostic-setup")

            timing = root / "supervisor-timing.jsonl"
            self.assertEqual(
                subprocess.run(
                    [sys.executable, str(TIMING), "project", "--allow-partial", "--path", str(timing),
                     "--output", str(projection), "--expected-run-id", "12345", "--expected-attempt", "1",
                     "--expected-source-sha", "a" * 40, "--expected-image-fingerprint", "b" * 32,
                     "--require-phase", "browser-setup", "--require-phase", "browser-initial",
                     "--require-supervisor-phase", "archive", "--require-supervisor-phase", "evidence-scan",
                     "--require-supervisor-phase", "upload"],
                    check=False,
                ).returncode,
                0,
            )
            timing_value = json.loads(projection.read_text(encoding="utf-8"))
            self.assertEqual(timing_value["phases"][0]["trust"], "supervisor")
            self.assertEqual(timing_value["phases"][0]["clock_domain"], "guest-startup")
            self.assertEqual(timing_value["phases"][0]["outcome"], "failed")
            self.assertIsInstance(timing_value["phases"][0]["duration_ms"], int)
            output = root / "bundle"
            archive = root / "bundle.tar.gz"
            logs = {
                "setup-log": setup_log,
                "image-build-log": root / "image.log",
                "npm-log": root / "npm.log",
                "playwright-log": root / "playwright.log",
                "browser-container-log": root / "container.log",
            }
            for path in logs.values():
                path.write_text("HEPH_GCP_DIAGNOSTIC status=failed reason=downstream\n", encoding="utf-8")
            self.assertEqual(
                COLLECTOR.collect(
                    output,
                    [f"first-failure={first_failure}", *[f"{label}={path}" for label, path in logs.items()], f"phase-timing={projection}"],
                    None, None, archive,
                ),
                0,
            )
            extracted = root / "downloaded"
            with tarfile.open(archive, "r:gz") as bundle:
                bundle.extractall(extracted)
            retained = extracted / "cooking-diagnostics"
            self.assertEqual(SCANNER.main([str(retained)]), 0)
            triage = SUMMARY.summarize(retained)
            self.assertEqual(triage["firstFailure"]["exit_code"], 78)
            self.assertEqual(triage["phaseTiming"]["status"], "partial")
            self.assertEqual(triage["phaseTiming"]["missingPhases"], ["browser-initial"])

    def test_explicit_log_with_secret_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-log-scan-") as raw:
            root = Path(raw)
            source = root / "npm.log"
            source.write_bytes(SCANNER.VALUES[0] + b"\n")
            serial = root / "serial.log"
            serial.write_text("HEPH_GCP_COOKING event=workload-result operation=cooking-workload phase=cooking status=failed exit_code=1\n", encoding="utf-8")
            output = root / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"serial={serial}", f"npm-log={source}"], None, None, None), 0
            )
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["collectionStatus"], "partial")
            self.assertEqual(manifest["rejectedSources"][0]["label"], "npm-log")
            self.assertFalse((output / "sources" / "npm-log").exists())

    def test_workflow_has_controller_only_injection_and_safe_summary(self) -> None:
        workflow = (ROOT.parent / ".github" / "workflows" / "cooking-e2e.yml").read_text(encoding="utf-8")
        self.assertIn("diagnostic_failure:", workflow)
        self.assertIn("GCP_DIAGNOSTIC_FAILURE:", workflow)
        self.assertIn("Publish safe GCP failure summary", workflow)
        self.assertIn("GITHUB_STEP_SUMMARY", workflow)

    def test_workflow_summary_renders_partial_and_complete_statuses(self) -> None:
        workflow = (ROOT.parent / ".github" / "workflows" / "cooking-e2e.yml").read_text(encoding="utf-8")
        body = workflow.split("python3 - <<'PY'\n", 1)[1].split("\n          PY", 1)[0]
        body = textwrap.dedent(body)
        with tempfile.TemporaryDirectory(prefix="heph-summary-render-") as raw:
            root = Path(raw)
            status = root / "status.json"
            summary = root / "summary.md"
            base = {
                "cleanup": "verified-absent", "upload": "verified-by-download", "download": "passed", "scan": "passed",
                "triage": {
                    "firstFailure": {"phase": "browser-setup", "command_id": "browser-setup", "exit_code": 78, "diagnostic_error": "setup-failed"},
                    "phaseTiming": {"status": "partial", "primaryFailurePhase": "browser-setup", "downstreamMissingPhases": ["browser-initial"],
                                     "durations": [{"phase": "browser-setup", "durationMs": 12, "outcome": "failed"}]},
                },
            }
            env = {
                **__import__("os").environ,
                "GCP_DIAGNOSTICS_STATUS": str(status), "GITHUB_STEP_SUMMARY": str(summary),
                "GCP_MODE": "gcp-cooking", "GCP_SCENARIO": "session-chat",
                "GCP_WORKLOAD_SHA": "a" * 40, "GCP_CONTROLLER_SHA": "b" * 40,
                "GCP_RUN_URL": "https://github.example/run/1",
                "GCP_CONTROLLER_TIMING": str(root / "controller-timing.json"),
            }
            (root / "controller-timing.json").write_text(json.dumps({
                "schema": 1,
                "completeness": "complete",
                "phases": [{"phase": "archive", "duration_ms": 23, "outcome": "passed"}],
            }), encoding="utf-8")
            for timing_status in ("partial", "complete"):
                base["triage"]["phaseTiming"]["status"] = timing_status
                status.write_text(json.dumps(base), encoding="utf-8")
                result = subprocess.run([sys.executable, "-c", body], env=env, text=True, capture_output=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                rendered = summary.read_text(encoding="utf-8")
                self.assertIn(f"`{timing_status}`", rendered)
                self.assertIn("browser-setup", rendered)
                self.assertIn("controller timings", rendered)
                self.assertIn("controller `archive`", rendered)

    def test_diagnostic_gate_acceptance_binds_42_and_78_expectations(self) -> None:
        source = (ROOT / "gcp-kvm-smoke.sh").read_text(encoding="utf-8")
        body = source.split("<<'PYGATE_ACCEPT' || gate_acceptance_status=$?\n", 1)[1].split("\nPYGATE_ACCEPT", 1)[0]
        with tempfile.TemporaryDirectory(prefix="heph-diagnostic-gate-") as raw:
            root = Path(raw)
            evidence = root / "sources" / "evidence-scan"
            evidence.parent.mkdir()
            evidence.write_text(json.dumps({"status": "failed", "rule": "browser-secret-org"}), encoding="utf-8")
            (root / "manifest.json").write_text(json.dumps({
                "rejectedSources": [{"label": "runtime-log", "reason": "credential-scan-rejected"}],
            }), encoding="utf-8")
            gate = {
                "test_mode": "diagnostic", "overall_exit_code": 42, "supervisor_exit_code": 42,
                "gates": {
                    "workload": {"state": "failed", "exit_code": 42},
                    "evidence-scan": {"state": "failed", "exit_code": 1, "reason_class": "evidence-scan-failed"},
                    "browser-validation": {"state": "failed", "exit_code": 42},
                },
            }
            # The embedded acceptance projection only reads these gate fields;
            # the collector has already validated the complete sidecar schema.
            (root / "sources" / "gate-results").write_text(json.dumps(gate), encoding="utf-8")
            for expected, overall, wanted in (("none", 42, 0), ("setup", 78, 0), ("none", 78, 1)):
                gate["overall_exit_code"] = overall
                gate["supervisor_exit_code"] = overall
                for name in ("workload", "browser-validation"):
                    gate["gates"][name]["exit_code"] = overall
                (root / "sources" / "gate-results").write_text(json.dumps(gate), encoding="utf-8")
                status = root / f"status-{expected}-{overall}.json"
                status.write_text(json.dumps({}), encoding="utf-8")
                result = subprocess.run(
                    [sys.executable, "-c", body, str(status), str(root), "diagnostic", "passed", expected],
                    text=True, capture_output=True, check=False,
                )
                self.assertEqual(result.returncode, wanted, result.stderr)
                value = json.loads(status.read_text(encoding="utf-8"))
                self.assertEqual(value["gateAcceptance"], "passed" if wanted == 0 else "failed")

    def test_diagnostic_stages_real_trusted_timing_helper(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-diagnostic-staging-") as raw:
            root = Path(raw)
            staging = root / "metadata"
            collector = ROOT / "collect-cooking-diagnostics.py"
            scanner = ROOT / "check-browser-evidence.py"
            gate = ROOT / "cooking-gate-results.py"
            capture = r'''
source "$1"
test_mode=diagnostic
diagnostics_enabled=true
collector_source="$3"
scanner_source="$4"
timing_source="$5"
gate_source="$6"
metadata_value() {
  case "$1" in
    diagnostics-collector-script) cat "$collector_source" ;;
    diagnostics-scanner-script) cat "$scanner_source" ;;
    phase-timing-script) cat "$timing_source" ;;
    phase-timing-script-sha256) sha256sum "$timing_source" | awk '{print $1}' ;;
    cooking-gate-results-helper) cat "$gate_source" ;;
    cooking-gate-results-script-sha256) sha256sum "$gate_source" | awk '{print $1}' ;;
    *) return 1 ;;
  esac
}
stage_diagnostics_metadata
diagnostics_enabled=false
test_mode=smoke
test -f "$trusted_phase_timing_script"
cmp -s "$trusted_phase_timing_script" "$timing_source"
'''
            result = subprocess.run(
                ["bash", "-Eeuo", "pipefail", "-c", capture, "stage", str(ROOT / "gcp-kvm-startup.sh"), str(staging),
                 str(collector), str(scanner), str(TIMING), str(gate)],
                env={**__import__("os").environ, "HEPH_GCP_STARTUP_LIBRARY": "1",
                     "HEPH_GCP_DIAGNOSTICS_METADATA_ROOT": str(staging)},
                text=True, capture_output=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
