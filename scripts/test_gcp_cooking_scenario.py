"""Contract checks for the allowlisted GCP Cooking scenario selector."""

from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent
WORKFLOW = ROOT.parent / ".github" / "workflows" / "cooking-e2e.yml"
PHASE_TIMING = ROOT / "gcp_phase_timing.py"

SESSION_PHASES = (
    "browser-setup",
    "runtime-guest-build",
    "oci-image-materialization",
    "gateway-services-ready",
    "runtime-worker-build",
    "gateway-readiness",
    "golden-tests",
    "database-tests",
    "browser-initial",
    "browser-recovery",
    "browser-concurrency",
)
SESSION_BROWSER_PHASES = ("browser-initial", "browser-recovery", "browser-concurrency")
COOKING_PHASES = (
    "dependency-setup",
    "production-project-build",
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
)
PHASE_DOMAINS = {
    "dependency-setup": "workload",
    "browser-setup": "workload",
    "production-project-build": "workload-libkrun",
    "runtime-guest-build": "workload-libkrun",
    "runtime-worker-build": "workload-libkrun",
    "oci-image-materialization": "workload-libkrun",
    "gateway-edge-ready": "workload-gateway",
    "gateway-services-ready": "workload-libkrun",
    "gateway-readiness": "workload-libkrun",
    "oci-builder": "workload-libkrun",
    "oci-verifier": "workload-libkrun",
    "golden-tests": "workload-libkrun",
    "database-tests": "workload-libkrun",
    "browser-initial": "workload-libkrun",
    "browser-recovery": "workload-libkrun",
    "browser-concurrency": "workload-libkrun",
    "browser-post-operation": "workload-libkrun",
}


class GcpCookingScenarioContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")
        cls.smoke = (ROOT / "gcp-kvm-smoke.sh").read_text(encoding="utf-8")
        cls.startup = (ROOT / "gcp-kvm-startup.sh").read_text(encoding="utf-8")
        cls.runtime = (ROOT / "gcp-cooking-run.sh").read_text(encoding="utf-8")

    def test_workflow_exposes_default_and_allowlisted_input(self) -> None:
        self.assertRegex(
            self.workflow,
            re.compile(
                r"cooking_scenario:\n"
                r"\s+description: .*\n"
                r"\s+required: true\n"
                r"\s+type: choice\n"
                r"\s+options: \[cooking, session-chat\]\n"
                r"\s+default: cooking"
            ),
        )
        self.assertIn(
            "GCP_COOKING_SCENARIO: ${{ inputs.cooking_scenario || 'cooking' }}",
            self.workflow,
        )

    def test_controller_validates_and_publishes_scenario_metadata(self) -> None:
        self.assertIn(
            "GCP_COOKING_SCENARIO must be cooking or session-chat", self.smoke
        )
        self.assertIn(
            'metadata_values+=",cooking-scenario=${cooking_scenario}"', self.smoke
        )
        self.assertIn(
            "cooking-scenario metadata must be cooking or session-chat", self.startup
        )
        self.assertIn(
            'HEPH_GCP_COOKING_SCENARIO="$cooking_scenario"', self.startup
        )
        self.assertIn(
            'cooking_scenario="${GCP_COOKING_SCENARIO:-cooking}"', self.smoke
        )

    def test_session_chat_selects_standalone_runner_without_cooking_e2e(self) -> None:
        session_start = self.runtime.index(
            'if [[ "$selected_cooking_scenario" == session-chat ]]; then'
        )
        cooking_start = self.runtime.index(
            'else\n    workload_scenario_env=(', session_start
        )
        session_branch = self.runtime[session_start:cooking_start]
        self.assertNotIn("workload_script=", session_branch)
        for flag in (
            "HEPHAESTUS_APP_SESSION_CHAT_E2E=1",
            "HEPHAESTUS_APP_LIBKRUN_E2E=1",
            "HEPHAESTUS_APP_COOKING_BUILD_PROOF=1",
            "HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E=1",
            "HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E=1",
            "HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E=1",
            "HEPHAESTUS_COOKING_BROWSER_E2E=1",
        ):
            self.assertIn(flag, session_branch)
        self.assertIn("unset HEPHAESTUS_APP_COOKING_E2E", session_branch)
        self.assertIn("HEPHAESTUS_COOKING_SCENARIO=session-chat", session_branch)
        self.assertNotIn("HEPHAESTUS_APP_COOKING_E2E=1", session_branch)
        self.assertNotIn("phase_timing_path=''", session_branch)
        self.assertIn('--scenario "$cooking_scenario" --require-complete-journey', self.runtime)
        self.assertIn(
            '--setenv=HEPH_GCP_PHASE_TIMING_PATH="$phase_timing_path"',
            self.runtime,
        )

        cooking_branch = self.runtime[cooking_start:]
        self.assertNotIn("HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E=1", cooking_branch)
        self.assertIn(
            'workload_scenario_env=(\n        "--setenv=HEPHAESTUS_APP_COOKING_E2E=1"',
            cooking_branch,
        )
        self.assertIn('exec "$1/examples/cooking/run.sh"', self.runtime)

    def test_phase_projection_requires_session_phases_and_retains_cooking_profile(self) -> None:
        startup = (ROOT / "gcp-kvm-startup.sh").read_text(encoding="utf-8")
        smoke = (ROOT / "gcp-kvm-smoke.sh").read_text(encoding="utf-8")
        self.assertIn('phase_timing_required_args+=(--require-phase "$required_phase")', startup)
        self.assertIn(
            'phase_timing_workload_required_args+=(--require-workload-phase "$required_phase")',
            startup,
        )
        session_start = startup.index(
            'if [[ "$selected_cooking_scenario" == session-chat ]]; then'
        )
        session_end = startup.index("  else", session_start)
        session_startup = startup[session_start:session_end]
        for phase in SESSION_PHASES:
            self.assertIn(phase, session_startup)
        for phase in SESSION_PHASES:
            self.assertIn(f"--require-workload-phase {phase}", smoke)
        for phase in SESSION_BROWSER_PHASES:
            self.assertIn(f"--require-workload-phase {phase}", smoke)
        for phase in ("production-project-build", "gateway-edge-ready", "browser-initial", "browser-post-operation"):
            self.assertIn(f"--require-workload-phase {phase}", smoke)

        with tempfile.TemporaryDirectory(prefix="gcp-cooking-phase-contract-") as directory:
            root = Path(directory)
            complete = root / "complete.jsonl"
            self.write_phase_records(complete, SESSION_PHASES)
            session_output = root / "session.json"
            session_result = self.project_phase_records(complete, session_output, SESSION_PHASES)
            self.assertEqual(session_result.returncode, 0, session_result.stderr)

            missing = root / "missing.jsonl"
            self.write_phase_records(missing, SESSION_PHASES[:-1])
            missing_output = root / "missing.json"
            missing_result = self.project_phase_records(missing, missing_output, SESSION_PHASES)
            self.assertNotEqual(missing_result.returncode, 0)

            cooking = root / "cooking.jsonl"
            self.write_phase_records(cooking, COOKING_PHASES)
            cooking_output = root / "cooking.json"
            cooking_result = self.project_phase_records(cooking, cooking_output, COOKING_PHASES)
            self.assertEqual(cooking_result.returncode, 0, cooking_result.stderr)

    def test_session_phase_profile_matches_real_emitter_boundaries(self) -> None:
        golden = (ROOT.parent / "crates/hephaestus-app/tests/golden.rs").read_text(
            encoding="utf-8"
        )
        libkrun = (ROOT / "run-libkrun-integration.sh").read_text(encoding="utf-8")
        wrapper = (ROOT.parent / "examples/cooking/run.sh").read_text(encoding="utf-8")
        for phase in SESSION_PHASES:
            if phase in SESSION_BROWSER_PHASES:
                # These browser timers are emitted by the session-chat composed
                # browser owner; this contract test only covers existing
                # runner boundaries and phase requirements.
                continue
            if phase == "browser-setup":
                self.assertIn("phase_timing_start browser-setup", wrapper)
            elif phase == "gateway-readiness":
                self.assertIn(
                    'WorkloadPhaseTimer::start("gateway-readiness", workload_phase_timing)',
                    golden,
                )
            else:
                self.assertIn(f"phase_timing_start {phase}", libkrun)

    @staticmethod
    def write_phase_records(path: Path, phases: tuple[str, ...]) -> None:
        common = {
            "--path": str(path),
            "--trust": "workload",
            "--run-id": "1",
            "--attempt": "1",
            "--source-sha": "a" * 40,
            "--image-fingerprint": "b" * 32,
        }
        for phase in phases:
            domain = PHASE_DOMAINS[phase]
            start = subprocess.run(
                [
                    "python3",
                    str(PHASE_TIMING),
                    "start",
                    *sum(([key, value] for key, value in common.items()), []),
                    "--phase",
                    phase,
                    "--clock-domain",
                    domain,
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            if start.returncode:
                raise AssertionError(start.stderr)
            end = subprocess.run(
                [
                    "python3",
                    str(PHASE_TIMING),
                    "end",
                    *sum(([key, value] for key, value in common.items()), []),
                    "--phase",
                    phase,
                    "--clock-domain",
                    domain,
                    "--outcome",
                    "passed",
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            if end.returncode:
                raise AssertionError(end.stderr)

    @staticmethod
    def project_phase_records(path: Path, output: Path, phases: tuple[str, ...]) -> subprocess.CompletedProcess[str]:
        arguments = [
            "python3",
            str(PHASE_TIMING),
            "project",
            "--path",
            str(path),
            "--output",
            str(output),
            "--expected-run-id",
            "1",
            "--expected-attempt",
            "1",
            "--expected-source-sha",
            "a" * 40,
            "--expected-image-fingerprint",
            "b" * 32,
        ]
        for phase in phases:
            arguments.extend(("--require-workload-phase", phase))
        return subprocess.run(arguments, capture_output=True, text=True, check=False)

    def test_runtime_rejects_unknown_scenario_before_workload(self) -> None:
        result = subprocess.run(
            ["bash", str(ROOT / "gcp-cooking-run.sh")],
            env={**os.environ, "HEPH_GCP_COOKING_SCENARIO": "unknown"},
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "HEPH_GCP_COOKING_SCENARIO must be cooking or session-chat",
            result.stderr,
        )

    def run_wrapper_stub(self, scenario: str) -> dict[str, str]:
        with tempfile.TemporaryDirectory(prefix="gcp-cooking-scenario-") as root_name:
            root = Path(root_name)
            scripts = root / "scripts"
            cooking = root / "examples" / "cooking"
            fake_bin = root / "bin"
            scripts.mkdir(parents=True)
            cooking.mkdir(parents=True)
            (root / "e2e" / "playwright").mkdir(parents=True)
            fake_bin.mkdir()
            (fake_bin / "node").write_text(
                "#!/usr/bin/env bash\n"
                "trap 'exit 0' TERM INT\n"
                "while :; do sleep 0.1; done\n",
                encoding="utf-8",
            )
            (fake_bin / "npm").write_text(
                "#!/usr/bin/env bash\nexit 0\n", encoding="utf-8"
            )
            (fake_bin / "curl").write_text(
                "#!/usr/bin/env bash\nexit 0\n", encoding="utf-8"
            )
            (scripts / "run-ui-e2e-host-bridge.sh").write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "bridge_dir=$1\n"
                ": >\"$bridge_dir/.ready\"\n"
                "trap 'exit 0' TERM INT\n"
                "while :; do sleep 0.1; done\n",
                encoding="utf-8",
            )
            shutil.copy2(ROOT.parent / "examples/cooking/run.sh", cooking / "run.sh")
            shutil.copy2(ROOT / "shell-failure-diagnostics.sh", scripts)
            shutil.copy2(ROOT / "check-browser-evidence.py", scripts)
            (scripts / "run-libkrun-integration.sh").write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "printf 'runner=session-chat\\n' >\"$SCENARIO_CAPTURE\"\n"
                "printf 'session=%s\\n' \"${HEPHAESTUS_APP_SESSION_CHAT_E2E-unset}\" >>\"$SCENARIO_CAPTURE\"\n"
                "printf 'libkrun=%s\\n' \"${HEPHAESTUS_APP_LIBKRUN_E2E-unset}\" >>\"$SCENARIO_CAPTURE\"\n"
                "printf 'build=%s\\n' \"${HEPHAESTUS_APP_COOKING_BUILD_PROOF-unset}\" >>\"$SCENARIO_CAPTURE\"\n"
                "printf 'browser=%s\\n' \"${HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E-unset}\" >>\"$SCENARIO_CAPTURE\"\n"
                "printf 'concurrent=%s\\n' \"${HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E-unset}\" >>\"$SCENARIO_CAPTURE\"\n"
                "printf 'cooking=%s\\n' \"${HEPHAESTUS_APP_COOKING_E2E-unset}\" >>\"$SCENARIO_CAPTURE\"\n",
                encoding="utf-8",
            )
            (cooking / "preflight.sh").write_text(
                "#!/usr/bin/env bash\nprintf 'preflight=ok\\n' >\"$SCENARIO_CAPTURE\"\n",
                encoding="utf-8",
            )
            (scripts / "run-gateway-libkrun-e2e.sh").write_text(
                "#!/usr/bin/env bash\n"
                "printf 'runner=cooking\\n' >\"$SCENARIO_CAPTURE\"\n"
                "printf 'cooking=%s\\n' \"${HEPHAESTUS_APP_COOKING_E2E-unset}\" >>\"$SCENARIO_CAPTURE\"\n",
                encoding="utf-8",
            )
            for path in (
                cooking / "run.sh",
                scripts / "run-libkrun-integration.sh",
                cooking / "preflight.sh",
                scripts / "run-gateway-libkrun-e2e.sh",
                scripts / "run-ui-e2e-host-bridge.sh",
                fake_bin / "node",
                fake_bin / "npm",
                fake_bin / "curl",
            ):
                path.chmod(0o755)
            capture = root / "capture.txt"
            temp_root = root / "tmp"
            libkrun_tmp_root = root / "libkrun-tmp"
            diagnostics = root / "diagnostics"
            temp_root.mkdir()
            libkrun_tmp_root.mkdir()
            diagnostics.mkdir()
            environment = {
                **os.environ,
                "HEPHAESTUS_COOKING_SESSION_DETACHED": "1",
                "HEPHAESTUS_COOKING_SCENARIO": scenario,
                "HEPHAESTUS_COOKING_BROWSER_E2E": "0",
                "HEPHAESTUS_COOKING_SOURCE_ROOT": str(cooking),
                "HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE": "registry.invalid/python@sha256:" + "a" * 64,
                "HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE": "registry.invalid/rust@sha256:" + "b" * 64,
                "HEPHAESTUS_COOKING_TIMEOUT_SECONDS": "30",
                "HEPHAESTUS_COOKING_DIAGNOSTICS_DIR": str(diagnostics),
                "TMPDIR": str(temp_root),
                "HEPHAESTUS_LIBKRUN_TMP_ROOT": str(libkrun_tmp_root),
                "SCENARIO_CAPTURE": str(capture),
                "HEPHAESTUS_APP_COOKING_E2E": "inherited",
                "HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E": (
                    "1" if scenario == "session-chat" else "0"
                ),
                "PATH": f"{fake_bin}:{os.environ['PATH']}",
            }
            completed = subprocess.run(
                ["bash", str(cooking / "run.sh")],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            return dict(
                line.split("=", 1)
                for line in capture.read_text(encoding="utf-8").splitlines()
            )

    def test_wrapper_session_chat_captures_standalone_flags(self) -> None:
        captured = self.run_wrapper_stub("session-chat")
        self.assertEqual(
            captured,
            {
                "runner": "session-chat",
                "session": "1",
                "libkrun": "1",
                "build": "1",
                "browser": "1",
                "concurrent": "1",
                "cooking": "unset",
            },
        )

    def test_wrapper_default_cooking_captures_existing_flag(self) -> None:
        captured = self.run_wrapper_stub("cooking")
        self.assertEqual(captured, {"runner": "cooking", "cooking": "1"})


if __name__ == "__main__":
    unittest.main()
