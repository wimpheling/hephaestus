"""Functional coverage for safe shell-runner failure boundaries."""

from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).parent
HELPER = ROOT / "shell-failure-diagnostics.sh"
COLLECTOR = ROOT / "collect-cooking-diagnostics.py"
SUMMARIZER = ROOT / "summarize-cooking-diagnostics.py"
MARKER_RE = re.compile(
    r"^HEPH_GCP_SHELL_FAILURE script=(?P<script>[a-z0-9-]+) "
    r"component=(?P<component>[a-z0-9-]+) operation=(?P<operation>[a-z0-9-]+) "
    r"reason=(?P<reason>[a-z-]+) exit_code=(?P<exit>[0-9]+) line=(?P<line>[0-9]+)$"
)


class ShellFailureDiagnosticsTests(unittest.TestCase):
    def _run(self, body: str) -> subprocess.CompletedProcess[str]:
        script = f"""#!/usr/bin/env bash
set -Eeuo pipefail
source {HELPER!s}
heph_shell_failure_init libkrun-integration libkrun
{body}
"""
        return subprocess.run(["bash", "-c", script], text=True, capture_output=True, check=False)

    def _marker(self, result: subprocess.CompletedProcess[str]) -> dict[str, str]:
        lines = [line for line in result.stderr.splitlines() if line.startswith("HEPH_GCP_SHELL_FAILURE ")]
        self.assertEqual(len(lines), 1, result.stderr)
        match = MARKER_RE.fullmatch(lines[0])
        self.assertIsNotNone(match, lines[0])
        assert match is not None
        self.assertNotIn("fixture-secret", lines[0])
        self.assertNotIn("BASH_COMMAND", lines[0])
        return match.groupdict()

    def test_failure_boundaries_preserve_status_and_classify_input_vs_assertion(self):
        cases = (
            ("false;", 1, "command", "command-failed"),
            ("heph_shell_failure_die cgroup-events missing-input 7 \"$LINENO\"; exit 7", 7, "cgroup-events", "missing-input"),
            ("heph_shell_failure_die network-integrity network-mismatch 1 \"$LINENO\"; exit 1", 1, "network-integrity", "network-mismatch"),
            ("heph_shell_failure_die cgroup-cleanup assertion-mismatch 1 \"$LINENO\"; exit 1", 1, "cgroup-cleanup", "assertion-mismatch"),
        )
        for body, status, operation, reason in cases:
            with self.subTest(operation=operation):
                result = self._run(f"printf 'passed test\\n'\n{body}")
                self.assertEqual(result.returncode, status)
                marker = self._marker(result)
                self.assertEqual(marker["operation"], operation)
                self.assertEqual(marker["reason"], reason)
                self.assertEqual(int(marker["exit"]), status)

    def test_success_and_signal_emit_no_secret_or_argument_data(self):
        success = self._run(
            """cleanup() { local status=$?; trap - EXIT; heph_shell_failure_on_exit "$status" "$LINENO"; heph_shell_failure_begin_cleanup; exit "$status"; }
trap cleanup EXIT
printf 'success\\n'
"""
        )
        self.assertEqual(success.returncode, 0)
        self.assertNotIn("HEPH_GCP_SHELL_FAILURE", success.stderr)

        signal_result = self._run(
            """cleanup() { local status=$?; trap - EXIT; heph_shell_failure_on_exit "$status" "$LINENO"; heph_shell_failure_begin_cleanup; exit "$status"; }
trap cleanup EXIT
trap 'exit 143' TERM
secret_arg=fixture-secret
kill -TERM $$
"""
        )
        self.assertEqual(signal_result.returncode, 143)
        marker = self._marker(signal_result)
        self.assertEqual(marker["reason"], "signal")
        self.assertEqual(marker["exit"], "143")
        self.assertNotIn("fixture-secret", signal_result.stderr)

        zero = self._run('heph_shell_failure_emit command command-failed 0 "$LINENO"\n')
        self.assertEqual(zero.returncode, 0)
        self.assertNotIn("HEPH_GCP_SHELL_FAILURE", zero.stderr)

    def test_production_cleanup_and_network_read_checks_classify_input_errors(self):
        cleanup_check = self._run(
            """fixture_root=$(mktemp -d)
if [[ ! -d "$fixture_root/runtime" ]]; then
    heph_shell_failure_die runtime-cleanup missing-input 1 "$LINENO"
    exit 1
fi
runtime_entries=''
if ! runtime_entries="$(find "$fixture_root/runtime" -mindepth 1 -print -quit)"; then
    heph_shell_failure_die runtime-cleanup read-failed 1 "$LINENO"
    exit 1
fi
"""
        )
        self.assertEqual(cleanup_check.returncode, 1)
        marker = self._marker(cleanup_check)
        self.assertEqual((marker["operation"], marker["reason"]), ("runtime-cleanup", "missing-input"))

        cleanup_read = self._run(
            """fixture_root=$(mktemp -d)
mkdir "$fixture_root/runtime"
chmod 000 "$fixture_root/runtime"
runtime_entries=''
if ! runtime_entries="$(find "$fixture_root/runtime" -mindepth 1 -print -quit)"; then
    heph_shell_failure_die runtime-cleanup read-failed 1 "$LINENO"
    exit 1
fi
"""
        )
        self.assertEqual(cleanup_read.returncode, 1)
        marker = self._marker(cleanup_read)
        self.assertEqual((marker["operation"], marker["reason"]), ("runtime-cleanup", "read-failed"))

        cgroup_read = self._run(
            """cgroup_root=$(mktemp -d)
mkdir "$cgroup_root/cgroup.events"
if [[ ! -r "$cgroup_root/cgroup.events" ]]; then
    heph_shell_failure_die cgroup-events missing-input 1 "$LINENO"
    exit 1
fi
cgroup_events_status=0
if grep -q '^populated 0$' "$cgroup_root/cgroup.events"; then
    cgroup_events_status=0
else
    cgroup_events_status=$?
fi
if (( cgroup_events_status > 1 )); then
    heph_shell_failure_die cgroup-events read-failed 1 "$LINENO"
    exit 1
fi
if (( cgroup_events_status == 1 )); then
    heph_shell_failure_die cgroup-events assertion-mismatch 1 "$LINENO"
    exit 1
fi
"""
        )
        self.assertEqual(cgroup_read.returncode, 1)
        marker = self._marker(cgroup_read)
        self.assertEqual((marker["operation"], marker["reason"]), ("cgroup-events", "read-failed"))

        network_check = self._run(
            """network_snapshot() { python3 -c 'raise SystemExit(2)'; }
network_after=''
if ! network_after="$(network_snapshot)"; then
    heph_shell_failure_die network-integrity read-failed 1 "$LINENO"
    exit 1
fi
"""
        )
        self.assertEqual(network_check.returncode, 1)
        marker = self._marker(network_check)
        self.assertEqual((marker["operation"], marker["reason"]), ("network-integrity", "read-failed"))

    def test_real_marker_survives_collector_and_summary(self):
        result = self._run(
            """printf 'HEPH_GCP_TEST test=real_shell_boundary status=passed\\n'
heph_shell_failure_die cgroup-events missing-input 7 "$LINENO"
exit 7
"""
        )
        self.assertEqual(result.returncode, 7)
        marker = self._marker(result)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            serial = root / "serial.log"
            serial.write_text(result.stderr + "HEPH_GCP_TEST test=real_shell_boundary status=passed\n", encoding="utf-8")
            gates = root / "gate-results.json"
            gates.write_text(json.dumps({
                "schema": 1,
                "revision": "a" * 40,
                "script_sha256": "b" * 64,
                "test_mode": "gcp-cooking",
                "overall_exit_code": 7,
                "supervisor_exit_code": 7,
                "finalized": True,
                "gates": {
                    "workload": {"state": "failed", "exit_code": 7, "reason_class": "workload-failed"},
                    "evidence-scan": {"state": "passed", "exit_code": 0, "reason_class": "none"},
                    "browser-validation": {"state": "passed", "exit_code": 0, "reason_class": "none"},
                },
            }), encoding="utf-8")
            scan = root / "evidence-scan.json"
            scan.write_text(json.dumps({"schema": 1, "status": "passed", "rule": "none", "file_class": "none", "path_sha256": None, "checked_files": 1, "checked_bytes": 1}) + "\n", encoding="utf-8")
            browser = root / "browser-summary.json"
            browser.write_text(json.dumps({"status": "passed", "suite": "cooking-playwright", "test": "browser-journey", "phase": "browser", "report_state": "complete", "result_origin": "playwright-report", "counts": {"passed": 2, "failed": 0, "skipped": 0, "timed_out": 0}, "observed_phases": ["initial", "post-operation"], "passed_phases": ["initial", "post-operation"], "failure_metadata": []}) + "\n", encoding="utf-8")
            bundle = root / "bundle"
            collected = subprocess.run(
                [sys.executable, str(COLLECTOR), "--output-dir", str(bundle),
                 "--source", f"serial={serial}", "--source", f"gate-results={gates}",
                 "--source", f"evidence-scan={scan}", "--source", f"browser-summary={browser}"],
                text=True, capture_output=True, check=False,
            )
            self.assertEqual(collected.returncode, 0, collected.stderr)
            summary = subprocess.run([sys.executable, str(SUMMARIZER), str(bundle)], text=True, capture_output=True, check=False)
            self.assertEqual(summary.returncode, 0, summary.stderr)
            value = json.loads(summary.stdout)
            matches = [item for item in value["technicalContext"] if item.get("kind") == "shell-failure"]
            self.assertEqual(len(matches), 1, value["technicalContext"])
            self.assertEqual(matches[0]["operation"], marker["operation"])
            self.assertEqual(matches[0]["reason"], marker["reason"])
            self.assertEqual(matches[0]["exit_code"], 7)
            self.assertNotIn("fixture-secret", summary.stdout)

    def test_collector_rejects_zero_exit_failure_marker(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "serial.log"
            source.write_text(
                "HEPH_GCP_SHELL_FAILURE script=libkrun-integration component=libkrun "
                "operation=command reason=command-failed exit_code=0 line=4\n",
                encoding="utf-8",
            )
            result = subprocess.run(
                [sys.executable, str(COLLECTOR), "--output-dir", str(root / "bundle"), "--source", f"serial={source}"],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("collector-failure", result.stderr)


if __name__ == "__main__":
    unittest.main()
