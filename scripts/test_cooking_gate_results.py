"""Focused tests for the Cooking gate sidecar and runner gate transitions."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import os
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
HELPER = ROOT / "cooking-gate-results.py"
RUNNER = ROOT / "gcp-cooking-run.sh"


class CookingGateResultsTests(unittest.TestCase):
    def _invoke(self, path: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            ["python3", str(HELPER), "--path", str(path), *args],
            text=True,
            capture_output=True,
            check=False,
        )
        if check:
            self.assertEqual(result.returncode, 0, result.stderr)
        return result

    def _init(self, path: Path, mode: str = "gcp-cooking") -> None:
        digest = hashlib.sha256(HELPER.read_bytes()).hexdigest()
        self._invoke(
            path,
            "init",
            "--revision",
            "0123456789012345678901234567890123456789",
            "--script-sha256",
            digest,
            "--test-mode",
            mode,
        )

    def test_complete_failed_workload_preserves_independent_gate_states(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-gate-results-") as directory:
            parent = Path(directory)
            parent.chmod(0o700)
            path = parent / "cooking-gate-results.json"
            self._init(path)
            self._invoke(path, "begin", "workload")
            self._invoke(path, "complete", "workload", "--state", "failed", "--exit-code", "7", "--reason-class", "workload-failed")
            self._invoke(path, "begin", "evidence-scan")
            self._invoke(path, "complete", "evidence-scan", "--state", "passed", "--exit-code", "0", "--reason-class", "none")
            self._invoke(path, "begin", "browser-validation")
            self._invoke(path, "complete", "browser-validation", "--state", "passed", "--exit-code", "0", "--reason-class", "none")
            self._invoke(path, "finalize", "--overall-exit-code", "7")
            self._invoke(path, "finalize", "--overall-exit-code", "0", "--supervisor-exit-code", "7")
            value = json.loads(path.read_text(encoding="utf-8"))
            self.assertTrue(value["finalized"])
            self.assertEqual(value["overall_exit_code"], 7)
            self.assertEqual(value["supervisor_exit_code"], 7)
            self.assertEqual(value["gates"]["workload"], {"state": "failed", "exit_code": 7, "reason_class": "workload-failed"})
            self.assertEqual(value["gates"]["evidence-scan"]["state"], "passed")
            self.assertEqual(value["gates"]["browser-validation"]["state"], "passed")
            self.assertEqual((path.stat().st_mode & 0o777), 0o600)

    def test_finalize_marks_unfinished_gates_unknown_and_preserves_timeout(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-gate-results-") as directory:
            parent = Path(directory)
            parent.chmod(0o700)
            path = parent / "cooking-gate-results.json"
            self._init(path, "diagnostic")
            self._invoke(path, "begin", "workload")
            self._invoke(path, "complete", "workload", "--state", "timed-out", "--exit-code", "124", "--reason-class", "timeout")
            self._invoke(path, "finalize", "--overall-exit-code", "124")
            value = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(value["gates"]["workload"]["state"], "timed-out")
            for name in ("evidence-scan", "browser-validation"):
                self.assertEqual(value["gates"][name], {"state": "unknown", "exit_code": None, "reason_class": "unfinished"})

    def test_finalized_supervisor_exit_may_be_null_and_malformed_state_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-gate-results-") as directory:
            parent = Path(directory)
            parent.chmod(0o700)
            path = parent / "cooking-gate-results.json"
            self._init(path)
            self._invoke(path, "finalize", "--overall-exit-code", "1")
            value = json.loads(path.read_text(encoding="utf-8"))
            self.assertIsNone(value["supervisor_exit_code"])
            self.assertEqual(self._invoke(path, "validate").returncode, 0)
            value["schema"] = True
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self._invoke(path, "validate", check=False).returncode, 0)
            path.write_text("x" * 17_000, encoding="utf-8")
            self.assertNotEqual(self._invoke(path, "validate", check=False).returncode, 0)

    def test_validate_checks_helper_hash_and_rejects_symlink_target(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-gate-results-") as directory:
            parent = Path(directory)
            parent.chmod(0o700)
            path = parent / "cooking-gate-results.json"
            self._init(path)
            self.assertEqual(self._invoke(path, "validate").returncode, 0)
            other = parent / "other-helper.py"
            other.write_text("# different helper\n", encoding="utf-8")
            self.assertNotEqual(self._invoke(path, "--script-file", str(other), "validate", check=False).returncode, 0)
            path.unlink()
            target = parent / "target.json"
            target.write_text("keep\n", encoding="utf-8")
            path.symlink_to(target)
            self.assertNotEqual(self._invoke(path, "init", "--revision", "0123456789012345678901234567890123456789", "--script-sha256", hashlib.sha256(HELPER.read_bytes()).hexdigest(), "--test-mode", "gcp-cooking", check=False).returncode, 0)
            self.assertEqual(target.read_text(encoding="utf-8"), "keep\n")

    def test_runner_init_anchors_hash_to_executed_runtime_script(self) -> None:
        runner = RUNNER.read_text(encoding="utf-8")
        self.assertIn('gate_results_script_file="${BASH_SOURCE[0]}"', runner)
        self.assertIn('sha256sum "$gate_results_script_file"', runner)
        self.assertIn('--script-file "$gate_results_script_file" init', runner)
        with tempfile.TemporaryDirectory(prefix="heph-gate-results-") as directory:
            parent = Path(directory)
            parent.chmod(0o700)
            path = parent / "cooking-gate-results.json"
            runner_hash = hashlib.sha256(RUNNER.read_bytes()).hexdigest()
            result = self._invoke(
                path,
                "--script-file",
                str(RUNNER),
                "init",
                "--revision",
                "0123456789012345678901234567890123456789",
                "--script-sha256",
                runner_hash,
                "--test-mode",
                "gcp-cooking",
            )
            self.assertEqual(result.returncode, 0)
            self.assertEqual(json.loads(path.read_text(encoding="utf-8"))["script_sha256"], runner_hash)

    def test_runtime_revision_lookup_allows_only_exact_forge_checkout(self) -> None:
        runner = RUNNER.read_text(encoding="utf-8")
        self.assertIn(
            'git -c "safe.directory=$checkout_root" -C "$checkout_root" rev-parse HEAD',
            runner,
        )
        with tempfile.TemporaryDirectory(prefix="heph-gate-ownership-") as directory:
            checkout = Path(directory) / "checkout"
            checkout.mkdir(mode=0o700)
            subprocess.run(["git", "-C", str(checkout), "init", "-q"], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(checkout),
                    "-c",
                    "user.email=hephaestus@example.invalid",
                    "-c",
                    "user.name=Hephaestus",
                    "commit",
                    "--allow-empty",
                    "-qm",
                    "fixture",
                ],
                check=True,
            )
            expected = subprocess.run(
                ["git", "-C", str(checkout), "rev-parse", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            different_owner_env = {**os.environ, "GIT_TEST_ASSUME_DIFFERENT_OWNER": "1"}
            unsafe = subprocess.run(
                ["git", "-C", str(checkout), "rev-parse", "HEAD"],
                check=False,
                capture_output=True,
                text=True,
                env=different_owner_env,
            )
            self.assertEqual(unsafe.returncode, 128)
            self.assertIn("dubious ownership", unsafe.stderr)
            result = subprocess.run(
                [
                    "git",
                    "-c",
                    f"safe.directory={checkout}",
                    "-C",
                    str(checkout),
                    "rev-parse",
                    "HEAD",
                ],
                check=False,
                capture_output=True,
                text=True,
                env=different_owner_env,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), expected)

    def test_runner_segment_emits_workload_marker_for_exit_paths(self) -> None:
        runner_lines = RUNNER.read_text(encoding="utf-8").splitlines()
        self.assertIn("scan_status_report='/var/log/hephaestus/evidence-scan-status.json'", RUNNER.read_text(encoding="utf-8"))
        segment = "\n".join(runner_lines[next(i for i, line in enumerate(runner_lines) if line == "status=$?") :]) + "\n"
        with tempfile.TemporaryDirectory(prefix="heph-gate-segment-") as directory:
            root = Path(directory)
            checkout = root / "checkout" / "scripts"
            checkout.mkdir(parents=True)
            evidence = root / "evidence"
            evidence.mkdir()
            fake_bin = root / "bin"
            fake_bin.mkdir()
            (checkout / "cooking-gate-results.py").write_bytes(HELPER.read_bytes())
            (checkout / "check-browser-evidence.py").write_bytes((ROOT / "check-browser-evidence.py").read_bytes())
            (checkout / "project-playwright-browser-summary.py").write_bytes((ROOT / "project-playwright-browser-summary.py").read_bytes())
            (fake_bin / "systemctl").write_text(
                "#!/usr/bin/env bash\ncase \"${1:-}\" in stop|kill) exit 0;; "
                "show) printf 'ActiveState=inactive\\nSubState=dead\\nResult=failed\\nExecMainCode=exited\\nExecMainStatus=7\\nMainPID=0\\n';; esac\n",
                encoding="utf-8",
            )
            (fake_bin / "systemctl").chmod(0o700)
            for status in (0, 7, 124):
                run_evidence = root / f"evidence-{status}"
                run_evidence.mkdir()
                for index, (filename, title) in enumerate(
                    (
                        ("cooking-live-review.spec.ts", "cooking release install, mailbox, gateway configure, and binding"),
                        ("cooking-post-operation.spec.ts", "cooking post-operation controls, provenance, recovery, and denial"),
                    ),
                    1,
                ):
                    report_dir = run_evidence / f"browser.{index}"
                    report_dir.mkdir()
                    report = {
                        "config": {"rootDir": "/repo/e2e/playwright"},
                        "suites": [{"file": f"cooking-tests/{filename}", "title": "suite", "specs": [{
                            "file": f"cooking-tests/{filename}", "title": title,
                            "tests": [{"results": [{"status": "passed"}]}],
                        }]}],
                    }
                    (report_dir / "playwright-report.json").write_text(json.dumps(report), encoding="utf-8")
                sidecar = root / f"sidecar-{status}.json"
                digest = hashlib.sha256(HELPER.read_bytes()).hexdigest()
                self._init(sidecar)
                self._invoke(sidecar, "begin", "workload")
                prelude = f'''#!/usr/bin/env bash
set -Eeuo pipefail
export PATH={fake_bin}:$PATH
checkout_root={checkout.parent}
evidence_root={run_evidence}
browser_root={root}/browsers
cooking_unit=heph-test-cooking
gate_results_path={sidecar}
gate_results_helper={checkout}/cooking-gate-results.py
gate_results_initialized=true
gate_update() {{ python3 -B "$gate_results_helper" --path "$gate_results_path" "$@"; }}
remaining_seconds() {{ printf '600\\n'; }}
run_with_deadline() {{ "$@"; }}
run_with_collection_deadline() {{ "$@"; }}
phase_start() {{ printf 'HEPH_GCP_COOKING event=phase-start phase=%s\\n' "$1"; }}
phase_pass() {{ printf 'HEPH_GCP_COOKING event=phase-pass phase=%s\\n' "${{1:-evidence}}"; }}
finish() {{ local rc=$?; trap - EXIT; python3 -B "$gate_results_helper" --path "$gate_results_path" finalize --overall-exit-code "$rc" || true; exit "$rc"; }}
trap finish EXIT
set +e
if (( {status} == 124 )); then
  timeout --kill-after=1s 1s bash -c 'sleep 30'
else
  (exit {status})
fi
'''
                scan_path = "/var/log/hephaestus/evidence-scan-status.json"
                script = prelude + segment.replace(f"scan_status_report='{scan_path}'", f"scan_status_report='{run_evidence / 'evidence-scan-status.json'}'")
                script_path = root / f"segment-{status}.sh"
                script_path.write_text(script, encoding="utf-8")
                script_path.chmod(0o700)
                result = subprocess.run([str(script_path)], text=True, capture_output=True, check=False)
                self.assertEqual(result.returncode, status, result.stderr + result.stdout)
                output = result.stdout + result.stderr
                self.assertIn(f"event=workload-result operation=cooking-workload phase=cooking status={'passed' if status == 0 else 'failed'} exit_code={status}", output)
                self.assertIn("event=evidence-scan operation=evidence-scan phase=evidence status=passed exit_code=0", output)
                self.assertIn("event=browser-report-validation operation=browser-report-validation phase=evidence status=passed", output)
                value = json.loads(sidecar.read_text(encoding="utf-8"))
                self.assertTrue(value["finalized"])
                self.assertEqual(value["gates"]["workload"]["exit_code"], status)
                self.assertIsNone(value["supervisor_exit_code"])
                self.assertEqual(value["gates"]["workload"]["state"], "timed-out" if status == 124 else ("passed" if status == 0 else "failed"))


if __name__ == "__main__":
    unittest.main()
