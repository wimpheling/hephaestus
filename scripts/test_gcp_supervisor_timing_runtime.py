"""Regression coverage for the runtime cooking supervisor timing boundary."""

from __future__ import annotations

import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import time
import tempfile
import unittest


ROOT = Path(__file__).parent
STARTUP = ROOT / "gcp-kvm-startup.sh"
RUNTIME = ROOT / "gcp-cooking-run.sh"
COOKING = ROOT.parent / "examples" / "cooking" / "run.sh"
TIMING = ROOT / "gcp_phase_timing.py"
SOURCE = "a" * 40
IMAGE = "b" * 32


def _function_block(source: str, start_marker: str, end_marker: str) -> str:
    start = source.index(start_marker)
    end = source.index(end_marker, start)
    return source[start:end]


class GcpSupervisorTimingRuntimeTests(unittest.TestCase):
    def _run_cooking_caller(
        self,
        workload_status: int,
        timing_script: Path | None = None,
    ) -> tuple[subprocess.CompletedProcess[str], Path]:
        root = Path(tempfile.mkdtemp(prefix="heph-supervisor-timing-runtime-"))
        timing_path = root / "supervisor.jsonl"
        helper = timing_script or TIMING
        fake_bin = root / "bin"
        fake_bin.mkdir()
        fake_systemd = fake_bin / "systemd-run"
        fake_systemd.write_text(
            "#!/usr/bin/env bash\n"
            "set -Eeuo pipefail\n"
            "exit \"${HEPH_TEST_WORKLOAD_STATUS:?}\"\n",
            encoding="utf-8",
        )
        fake_systemd.chmod(0o700)

        startup_source = STARTUP.read_text(encoding="utf-8")
        runtime_source = RUNTIME.read_text(encoding="utf-8")
        startup_functions = _function_block(
            startup_source,
            "supervisor_phase_name() {",
            "metadata_value() {",
        )
        runtime_timing_functions = (
            _function_block(runtime_source, "runtime_phase_timing_outcome() {", "\nvalidate_sha256() {")
            + _function_block(runtime_source, "runtime_phase_name() {", "\nrequire_command() {")
        )
        workload_start = runtime_source.index("run_cooking_workload() {")
        workload_end = runtime_source.index("\n}\n", workload_start) + 2
        workload_function = runtime_source[workload_start:workload_end]
        result_start = runtime_source.index("workload_result='passed'")
        result_end = runtime_source.index("workload_gate_state='failed'", result_start)
        workload_result_block = runtime_source[result_start:result_end]

        # This follows the startup caller's phase boundary around the runtime
        # process.  The nested shell follows the runtime caller through the
        # actual run_cooking_workload function, evidence phase, and terminal
        # phase_pass.  systemd-run is the only substituted command and returns
        # the requested workload status, preserving the real status path.
        command = f'''#!/usr/bin/env bash
set -Eeuo pipefail
export PATH={fake_bin}:$PATH
revision={SOURCE}
run_id=98765
run_attempt=2
timing_image_fingerprint={IMAGE}
trusted_phase_timing_script={helper}
supervisor_phase_timing_path={timing_path}
install -m 0600 /dev/null "$supervisor_phase_timing_path"
declare -A supervisor_phase_timing_occurrence=()
supervisor_phase_timing_open=''
phase=initializing
marker() {{ :; }}
supervisor_phase_timing_outcome() {{
  case "$1" in
    124) printf '%s\\n' timed-out ;;
    130|143) printf '%s\\n' cancelled ;;
    *) printf '%s\\n' failed ;;
  esac
}}
{startup_functions}
phase_start gcp-cooking
set +e
(
  set -Eeuo pipefail
  runtime_phase_timing_script={helper}
  runtime_phase_timing_path={timing_path}
  runtime_phase_timing_image_fingerprint={IMAGE}
  gate_results_revision={SOURCE}
  HEPH_GCP_RUN_ID=98765
  GITHUB_RUN_ATTEMPT=2
  runtime_phase_timing_open=''
  declare -A runtime_phase_timing_occurrence=()
  phase=initializing
  fail() {{ return 1; }}
  {runtime_timing_functions}
  {workload_function}
  workload_started=true
  cooking_remaining=5
  cooking_timeout=4
  cooking_unit=heph-test-cooking
  forge_uid=10001
  forge_gid=10001
  checkout_root={root}
  runtime_python_ref=python-ref
  runtime_rust_ref=rust-ref
  workload_path=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
  rust_toolchain=1.88.0
  cache_root={root}/cache
  evidence_root={root}/evidence
  browser_root={root}/browsers
  workload_trust=trusted
  workload_home=/home/forge
  workload_cargo_home=/home/forge/.cargo
  workload_rustup_home=/home/forge/.rustup
  pr_sandbox_args=()
  phase_start cooking
  set +e
  run_cooking_workload
  status=$?
  set -e
  {workload_result_block}
  printf 'inner-status=%s\\n' "$status"
  printf 'gate-complete\\n'
  phase_start evidence
  phase_pass
  printf 'cleanup-after-evidence\\n'
  exit "$status"
)
status=$?
set -e
if ((status == 0)); then
  phase_pass
else
  supervisor_phase_timing_end "$(supervisor_phase_timing_outcome "$status")"
fi
printf 'outer-status=%s\\n' "$status"
exit "$status"
'''
        result = subprocess.run(
            ["bash", "-Eeuo", "pipefail", "-c", command, "timing-test"],
            env={**os.environ, "HEPH_TEST_WORKLOAD_STATUS": str(workload_status)},
            text=True,
            capture_output=True,
            check=False,
        )
        return result, root

    def test_real_cooking_caller_closes_supervisor_pairs_and_preserves_outcome(self) -> None:
        expected_outcomes = {0: "passed", 42: "failed", 124: "timed-out"}
        for status, expected_outcome in expected_outcomes.items():
            with self.subTest(status=status):
                result, root = self._run_cooking_caller(status)
                self.addCleanup(shutil.rmtree, root, True)
                self.assertEqual(result.returncode, status, result.stdout + result.stderr)
                self.assertIn(f"inner-status={status}", result.stdout)
                self.assertIn(f"outer-status={status}", result.stdout)

                records = [
                    json.loads(line)
                    for line in (root / "supervisor.jsonl").read_text(encoding="utf-8").splitlines()
                ]
                cooking = [
                    record
                    for record in records
                    if record["phase"] == "cooking-supervisor"
                    and record["clock_domain"] == "guest-runtime"
                    and record["occurrence"] == 1
                ]
                self.assertEqual([record["record"] for record in cooking], ["start", "end"])
                self.assertEqual(cooking[-1]["outcome"], expected_outcome)
                self.assertEqual(cooking[-1]["run_id"], "98765")
                self.assertEqual(cooking[-1]["attempt"], 2)
                self.assertEqual(cooking[-1]["source_sha"], SOURCE)
                evidence = [
                    record
                    for record in records
                    if record["phase"] == "evidence-scan"
                    and record["clock_domain"] == "guest-runtime"
                ]
                self.assertEqual([record["record"] for record in evidence], ["start", "end"])
                self.assertEqual(evidence[-1]["outcome"], "passed")

                validation = subprocess.run(
                    [
                        "python3",
                        str(TIMING),
                        "validate",
                        "--path",
                        str(root / "supervisor.jsonl"),
                        "--require-supervisor-phase",
                        "cooking-supervisor",
                        "--require-trust",
                        "supervisor",
                        "--expected-run-id",
                        "98765",
                        "--expected-attempt",
                        "2",
                        "--expected-source-sha",
                        SOURCE,
                        "--expected-image-fingerprint",
                        IMAGE,
                    ],
                    text=True,
                    capture_output=True,
                    check=False,
                )
                self.assertEqual(validation.returncode, 0, validation.stdout + validation.stderr)

    def test_inner_workload_sigterm_records_cancelled_and_exit_143(self) -> None:
        """A direct TERM must reach the inner timing EXIT trap with status 143."""
        runtime_source = COOKING.read_text(encoding="utf-8")
        run_cooking_start = runtime_source.index("run_cooking() {")
        command_start = runtime_source.index("bash -Eeuo pipefail -c '", run_cooking_start)
        command_end = runtime_source.index("' --", command_start)
        inner_script = runtime_source[
            command_start + len("bash -Eeuo pipefail -c '") : command_end
        ].replace("'\\''", "'")

        with tempfile.TemporaryDirectory(prefix="heph-inner-cooking-term-") as directory:
            root = Path(directory)
            script_dir = root / "script"
            script_dir.mkdir()
            preflight_started = root / "preflight.started"
            preflight = script_dir / "preflight.sh"
            preflight.write_text(
                f"#!/usr/bin/env bash\ntouch {preflight_started}\nsleep 30\n",
                encoding="utf-8",
            )
            preflight.chmod(0o700)
            cooking_root = root / "cooking"
            cooking_root.mkdir()
            repo_root = root / "repo"
            (repo_root / "scripts").mkdir(parents=True)
            timing_path = root / "timing.jsonl"
            process = subprocess.Popen(
                [
                    "bash",
                    "-Eeuo",
                    "pipefail",
                    "-c",
                    inner_script,
                    "cooking-inner-test",
                    str(script_dir),
                    str(cooking_root),
                    str(repo_root / "scripts"),
                    str(TIMING),
                ],
                env={
                    **os.environ,
                    "HEPH_GCP_PHASE_TIMING_PATH": str(timing_path),
                    "HEPH_GCP_PHASE_TIMING_SOURCE_SHA": SOURCE,
                    "HEPH_GCP_PHASE_TIMING_RUN_ID": "98765",
                    "HEPH_GCP_PHASE_TIMING_ATTEMPT": "2",
                },
                start_new_session=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                deadline = time.monotonic() + 5
                while not preflight_started.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(preflight_started.exists(), "inner workload did not start preflight")
                os.killpg(process.pid, signal.SIGTERM)
                stdout, stderr = process.communicate(timeout=10)
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)
            self.assertEqual(process.returncode, 143, stdout + stderr)
            records = [
                json.loads(line)
                for line in timing_path.read_text(encoding="utf-8").splitlines()
            ]
            self.assertEqual([record["record"] for record in records], ["start", "end"])
            self.assertEqual(records[-1]["phase"], "dependency-setup")
            self.assertEqual(records[-1]["outcome"], "cancelled")

            validation = subprocess.run(
                [
                    "python3",
                    str(TIMING),
                    "validate",
                    "--path",
                    str(timing_path),
                    "--require-workload-phase",
                    "dependency-setup",
                    "--expected-run-id",
                    "98765",
                    "--expected-attempt",
                    "2",
                    "--expected-source-sha",
                    SOURCE,
                ],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(validation.returncode, 0, validation.stdout + validation.stderr)

    def test_runtime_and_terminal_evidence_scans_survive_final_projection(self) -> None:
        """The real runtime scan must coexist with the required startup scan."""
        result, root = self._run_cooking_caller(0)
        self.addCleanup(shutil.rmtree, root, True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        timing = root / "supervisor.jsonl"
        projection = root / "final.json"

        def run_helper(*arguments: str) -> subprocess.CompletedProcess[str]:
            return subprocess.run(
                ["python3", str(TIMING), *arguments],
                text=True,
                capture_output=True,
                check=False,
            )

        required = (
            "--require-supervisor-phase", "archive",
            "--require-supervisor-phase", "evidence-scan",
            "--require-supervisor-phase", "upload",
            "--expected-run-id", "98765", "--expected-attempt", "2",
            "--expected-source-sha", SOURCE, "--expected-image-fingerprint", IMAGE,
        )
        for phase in ("archive", "evidence-scan", "upload"):
            common = (
                "--path", str(timing), "--phase", phase,
                "--trust", "supervisor", "--clock-domain", "guest-startup",
                "--run-id", "98765", "--attempt", "2", "--source-sha", SOURCE,
                "--image-fingerprint", IMAGE,
            )
            for command in (("start", *common), ("end", *common, "--outcome", "passed")):
                recorded = run_helper(*command)
                self.assertEqual(recorded.returncode, 0, recorded.stdout + recorded.stderr)

        projected = run_helper("project", "--path", str(timing), "--output", str(projection), *required)
        self.assertEqual(projected.returncode, 0, projected.stdout + projected.stderr)
        validated = run_helper("validate-projection", "--path", str(projection), *required)
        self.assertEqual(validated.returncode, 0, validated.stdout + validated.stderr)
        value = json.loads(projection.read_text(encoding="utf-8"))
        self.assertEqual(
            {item["clock_domain"] for item in value["phases"] if item["phase"] == "evidence-scan"},
            {"guest-runtime", "guest-startup"},
        )

        # The runtime scan cannot replace the terminal scan.  Other domains,
        # workload trust, and runtime archive timings remain unacceptable.
        for mutation in ("missing-startup", "wrong-domain", "workload-trust", "runtime-archive"):
            with self.subTest(mutation=mutation):
                changed = json.loads(json.dumps(value))
                if mutation == "missing-startup":
                    changed["phases"] = [
                        item for item in changed["phases"]
                        if not (item["phase"] == "evidence-scan" and item["clock_domain"] == "guest-startup")
                    ]
                else:
                    for item in changed["phases"]:
                        if mutation == "wrong-domain" and item["phase"] == "evidence-scan" and item["clock_domain"] == "guest-runtime":
                            item["clock_domain"] = "controller"
                        elif mutation == "workload-trust" and item["phase"] == "evidence-scan" and item["clock_domain"] == "guest-startup":
                            item.update(trust="workload", measurement="informational", clock_domain="workload")
                        elif mutation == "runtime-archive" and item["phase"] == "archive":
                            item["clock_domain"] = "guest-runtime"
                projection.write_text(json.dumps(changed), encoding="utf-8")
                rejected = run_helper("validate-projection", "--path", str(projection), *required)
                self.assertNotEqual(rejected.returncode, 0, rejected.stdout + rejected.stderr)

    def test_timing_end_failure_keeps_workload_exit_and_reaches_evidence(self) -> None:
        """A diagnostic write failure cannot replace the acceptance result."""
        for status in (42, 124):
            with self.subTest(status=status), tempfile.TemporaryDirectory(
                prefix="heph-timing-end-failure-"
            ) as directory:
                root = Path(directory)
                wrapper = root / "timing-wrapper.py"
                wrapper.write_text(
                    "import os, sys\n"
                    "if (sys.argv[1] == 'end' and '--phase' in sys.argv and\n"
                    "        sys.argv[sys.argv.index('--phase') + 1] == 'cooking-supervisor' and\n"
                    "        '--clock-domain' in sys.argv and\n"
                    "        sys.argv[sys.argv.index('--clock-domain') + 1] == 'guest-runtime'):\n"
                    "    raise SystemExit(77)\n"
                    f"os.execv(sys.executable, [sys.executable, {str(TIMING)!r}, *sys.argv[1:]])\n",
                    encoding="utf-8",
                )
                result, fixture_root = self._run_cooking_caller(
                    status, timing_script=wrapper
                )
                self.addCleanup(shutil.rmtree, fixture_root, True)
                self.assertEqual(result.returncode, status, result.stdout + result.stderr)
                self.assertIn("timing-helper-end-failed", result.stdout + result.stderr)
                self.assertIn("gate-complete", result.stdout)
                self.assertIn("cleanup-after-evidence", result.stdout)

                records = [
                    json.loads(line)
                    for line in (fixture_root / "supervisor.jsonl")
                    .read_text(encoding="utf-8")
                    .splitlines()
                ]
                runtime_cooking = [
                    record
                    for record in records
                    if record["phase"] == "cooking-supervisor"
                    and record["clock_domain"] == "guest-runtime"
                ]
                self.assertEqual([record["record"] for record in runtime_cooking], ["start"])
                runtime_evidence = [
                    record
                    for record in records
                    if record["phase"] == "evidence-scan"
                    and record["clock_domain"] == "guest-runtime"
                ]
                self.assertEqual([record["record"] for record in runtime_evidence], ["start", "end"])

                validation = subprocess.run(
                    [
                        "python3",
                        str(TIMING),
                        "validate",
                        "--path",
                        str(fixture_root / "supervisor.jsonl"),
                    ],
                    text=True,
                    capture_output=True,
                    check=False,
                )
                self.assertNotEqual(validation.returncode, 0)
                self.assertNotIn("77", validation.stdout + validation.stderr)


if __name__ == "__main__":
    unittest.main()
