"""Contract checks for the allowlisted GCP Cooking scenario selector."""

from pathlib import Path
import hashlib
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
    "browser-fork",
    "guest-negative-capability",
)
SESSION_BROWSER_PHASES = (
    "browser-initial",
    "browser-recovery",
    "browser-concurrency",
    "browser-fork",
)
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
    "browser-fork": "workload-libkrun",
    "guest-negative-capability": "workload-libkrun",
    "browser-post-operation": "workload-libkrun",
}


class GcpCookingScenarioContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")
        cls.smoke = (ROOT / "gcp-kvm-smoke.sh").read_text(encoding="utf-8")
        cls.startup = (ROOT / "gcp-kvm-startup.sh").read_text(encoding="utf-8")
        cls.runtime = (ROOT / "gcp-cooking-run.sh").read_text(encoding="utf-8")
        cls.cooking = (ROOT.parent / "examples/cooking/run.sh").read_text(
            encoding="utf-8"
        )

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
            "HEPHAESTUS_APP_SESSION_CHAT_NEGATIVE_E2E=1",
            "HEPHAESTUS_APP_LIBKRUN_E2E=1",
            "HEPHAESTUS_APP_COOKING_BUILD_PROOF=1",
            "HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E=1",
            "HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E=1",
            "HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E=1",
            "HEPHAESTUS_APP_SESSION_CHAT_FORK_E2E=1",
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
        self.assertIn(
            '--source "session-chat-negative-summary=${cooking_evidence_root}/session-chat-negative-summary.json"',
            startup,
        )
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

    def test_session_chat_bootstraps_owned_caddy_before_browser_setup(self) -> None:
        browser_setup = self.cooking.index(
            'if [[ "${HEPHAESTUS_COOKING_BROWSER_E2E:-1}" == "1" ]]'
        )
        caddy_reentry = self.cooking.index(
            '"${repo_root}/scripts/run-gateway-libkrun-e2e.sh"'
        )
        self.assertLess(caddy_reentry, browser_setup)

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

    def run_wrapper_stub(
        self,
        scenario: str,
        *,
        complete_caddy: bool = False,
        invalid_complete_marker: bool = False,
        expected_status: int = 0,
        explicit_browser_image: str | None = None,
        repeat: bool = False,
        build_fail: bool = False,
        unsafe_build_output: bool = False,
        reviewed_archive: bool = False,
        archive_hash_mismatch: bool = False,
        browser_e2e: bool = False,
    ) -> dict[str, str]:
        with tempfile.TemporaryDirectory(prefix="gcp-cooking-scenario-") as root_name:
            root = Path(root_name)
            scripts = root / "scripts"
            cooking = root / "examples" / "cooking"
            fake_bin = root / "bin"
            scripts.mkdir(parents=True)
            cooking.mkdir(parents=True)
            (root / "e2e" / "playwright").mkdir(parents=True)
            fake_bin.mkdir()
            podman_state = root / "podman-state"
            podman_state.mkdir()
            (fake_bin / "podman").write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "state=${PODMAN_STATE:?}\n"
                "if [[ \"${1:-}\" == image && \"${2:-}\" == exists ]]; then\n"
                "    [[ -f \"$state/image\" ]]\n"
                "    exit\n"
                "fi\n"
                "if [[ \"${1:-}\" == build ]]; then\n"
                "    count=0\n"
                "    [[ -f \"$state/build-count\" ]] && count=$(cat \"$state/build-count\")\n"
                "    printf '%s\\n' $((count + 1)) >\"$state/build-count\"\n"
                "    if [[ \"${PODMAN_BUILD_FAIL:-0}\" == 2 ]]; then\n"
                "        printf '%s\\n' 'cooking-inbound-only-fixture-sentinel' >&2\n"
                "        exit 17\n"
                "    elif [[ \"${PODMAN_BUILD_FAIL:-0}\" == 1 ]]; then\n"
                "        printf '%s\\n' 'reviewed image builder failed safely' >&2\n"
                "        exit 17\n"
                "    fi\n"
                "    touch \"$state/image\"\n"
                "    exit 0\n"
                "fi\n"
                "if [[ \"${1:-}\" == load ]]; then\n"
                "    touch \"$state/image\"\n"
                "    exit 0\n"
                "fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            image_script = scripts / "installed-ui-browser-image"
            image_script.mkdir()
            shutil.copy2(
                ROOT.parent / "scripts/installed-ui-browser-image/build.sh",
                image_script / "build.sh",
            )
            (image_script / "Dockerfile").write_text("FROM scratch\n", encoding="utf-8")
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
                "printf 'fork=%s\\n' \"${HEPHAESTUS_APP_SESSION_CHAT_FORK_E2E-unset}\" >>\"$SCENARIO_CAPTURE\"\n"
                "printf 'caddy-wrapper=%s\\n' \"${HEPHAESTUS_TEST_CADDY_WRAPPER-unset}\" >>\"$SCENARIO_CAPTURE\"\n"
                "printf 'caddy-port=%s\\n' \"${HEPHAESTUS_CADDY_TEST_PUBLIC_PORT-unset}\" >>\"$SCENARIO_CAPTURE\"\n"
                "printf 'installed-ui=%s\\n' \"${HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE-unset}\" >>\"$SCENARIO_CAPTURE\"\n"
                "if [[ -n \"${IMAGE_CAPTURE:-}\" ]]; then printf '%s\\n' \"${HEPHAESTUS_PLAYWRIGHT_IMAGE-unset}\" >\"$IMAGE_CAPTURE\"; fi\n"
                "printf 'cooking=%s\\n' \"${HEPHAESTUS_APP_COOKING_E2E-unset}\" >>\"$SCENARIO_CAPTURE\"\n",
                encoding="utf-8",
            )
            (scripts / "run-installed-ui-e2e.sh").write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "[[ \"${HEPHAESTUS_INSTALLED_UI_PREREQUISITE_ONLY:-0}\" == 1 ]]\n"
                ": >\"$PREREQUISITE_CAPTURE\"\n",
                encoding="utf-8",
            )
            (cooking / "preflight.sh").write_text(
                "#!/usr/bin/env bash\nprintf 'preflight=ok\\n' >\"$SCENARIO_CAPTURE\"\n",
                encoding="utf-8",
            )
            (scripts / "run-gateway-libkrun-e2e.sh").write_text(
                "#!/usr/bin/env bash\n"
                "if [[ \"${1:-}\" == -- ]]; then\n"
                "    shift\n"
                "    export HEPHAESTUS_TEST_CADDY_WRAPPER=1\n"
                "    caddy_ca=\"${TMPDIR}/fake-caddy-ca.pem\"\n"
                "    printf '%s\\n' 'fake-caddy-ca' >\"$caddy_ca\"\n"
                "    export HEPHAESTUS_CADDY_TEST_TLS=1\n"
                "    export HEPHAESTUS_CADDY_TEST_ADMIN_URL=http://127.0.0.1:41001\n"
                "    export HEPHAESTUS_CADDY_TEST_PUBLIC_URL=https://127.0.0.1:41002\n"
                "    export HEPHAESTUS_CADDY_TEST_LISTEN=127.0.0.1:41002\n"
                "    export HEPHAESTUS_CADDY_TEST_PUBLIC_PORT=41002\n"
                "    export HEPHAESTUS_CADDY_TEST_CA_CERT=\"$caddy_ca\"\n"
                "    export HEPHAESTUS_CADDY_TEST_ENV_COMPLETE=1\n"
                "    \"$@\"\n"
                "    exit\n"
                "fi\n"
                "printf 'runner=cooking\\n' >\"$SCENARIO_CAPTURE\"\n"
                "printf 'cooking=%s\\n' \"${HEPHAESTUS_APP_COOKING_E2E-unset}\" >>\"$SCENARIO_CAPTURE\"\n",
                encoding="utf-8",
            )
            for path in (
                cooking / "run.sh",
                scripts / "run-libkrun-integration.sh",
                scripts / "run-installed-ui-e2e.sh",
                cooking / "preflight.sh",
                scripts / "run-gateway-libkrun-e2e.sh",
                scripts / "run-ui-e2e-host-bridge.sh",
                image_script / "build.sh",
                fake_bin / "node",
                fake_bin / "npm",
                fake_bin / "curl",
                fake_bin / "podman",
            ):
                path.chmod(0o755)
            capture = root / "capture.txt"
            image_capture = root / "image-capture.txt"
            temp_root = root / "tmp"
            libkrun_tmp_root = root / "libkrun-tmp"
            diagnostics = root / "diagnostics"
            temp_root.mkdir()
            libkrun_tmp_root.mkdir()
            diagnostics.mkdir()
            external_ca = root / "external-caddy-ca.pem"
            external_ca.write_text("external-caddy-ca\n", encoding="utf-8")
            environment = {
                **os.environ,
                "HEPHAESTUS_COOKING_SESSION_DETACHED": "1",
                "HEPHAESTUS_COOKING_SCENARIO": scenario,
                "HEPHAESTUS_COOKING_BROWSER_E2E": "1" if browser_e2e else "0",
                "HEPHAESTUS_COOKING_SOURCE_ROOT": str(cooking),
                "HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE": "registry.invalid/python@sha256:" + "a" * 64,
                "HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE": "registry.invalid/rust@sha256:" + "b" * 64,
                "HEPHAESTUS_COOKING_TIMEOUT_SECONDS": "30",
                "HEPHAESTUS_COOKING_DIAGNOSTICS_DIR": str(diagnostics),
                "TMPDIR": str(temp_root),
                "HEPHAESTUS_LIBKRUN_TMP_ROOT": str(libkrun_tmp_root),
                "SCENARIO_CAPTURE": str(capture),
                "PODMAN_STATE": str(podman_state),
                "IMAGE_CAPTURE": str(image_capture),
                "PREREQUISITE_CAPTURE": str(root / "prerequisite-capture"),
                "HEPHAESTUS_APP_COOKING_E2E": "inherited",
                "HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E": (
                    "1" if scenario == "session-chat" else "0"
                ),
                "HEPHAESTUS_APP_SESSION_CHAT_FORK_E2E": (
                    "1" if scenario == "session-chat" else "0"
                ),
                "PATH": f"{fake_bin}:{os.environ['PATH']}",
            }
            if explicit_browser_image is not None:
                environment["HEPHAESTUS_PLAYWRIGHT_IMAGE"] = explicit_browser_image
            if reviewed_archive:
                archive = root / "installed-ui-browser-image.oci"
                archive.write_bytes(b"reviewed-installed-ui-image")
                environment["HEPHAESTUS_PLAYWRIGHT_IMAGE_ARCHIVE"] = str(archive)
                environment["HEPHAESTUS_PLAYWRIGHT_IMAGE_ARCHIVE_SHA256"] = hashlib.sha256(
                    archive.read_bytes()
                ).hexdigest()
                if archive_hash_mismatch:
                    environment["HEPHAESTUS_PLAYWRIGHT_IMAGE_ARCHIVE_SHA256"] = "0" * 64
            if build_fail:
                environment["PODMAN_BUILD_FAIL"] = "2" if unsafe_build_output else "1"
            if complete_caddy:
                environment.update(
                    {
                        "HEPHAESTUS_CADDY_TEST_TLS": "1",
                        "HEPHAESTUS_CADDY_TEST_ADMIN_URL": "http://127.0.0.1:42001",
                        "HEPHAESTUS_CADDY_TEST_PUBLIC_URL": "https://127.0.0.1:42002",
                        "HEPHAESTUS_CADDY_TEST_LISTEN": "127.0.0.1:42002",
                        "HEPHAESTUS_CADDY_TEST_PUBLIC_PORT": "42002",
                        "HEPHAESTUS_CADDY_TEST_CA_CERT": str(external_ca),
                    }
                )
            if invalid_complete_marker:
                environment["HEPHAESTUS_CADDY_TEST_ENV_COMPLETE"] = "1"
            completed = subprocess.run(
                ["bash", str(cooking / "run.sh")],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(completed.returncode, expected_status, completed.stderr)
            if repeat:
                repeated = subprocess.run(
                    ["bash", str(cooking / "run.sh")],
                    env=environment,
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertEqual(repeated.returncode, expected_status, repeated.stderr)
            build_count = podman_state / "build-count"
            if reviewed_archive:
                self.assertFalse(build_count.exists())
            elif (scenario == "session-chat" or browser_e2e) and explicit_browser_image is None and not invalid_complete_marker:
                self.assertTrue(build_count.is_file())
                self.assertEqual(build_count.read_text(encoding="utf-8").strip(), "1")
            else:
                self.assertFalse(build_count.exists())
            if build_fail:
                self.assertIn(
                    "Installed UI browser image preparation failed; retained diagnostics=",
                    completed.stderr,
                )
                self.assertIn(
                    "HEPH_GCP_FAILURE phase=browser-setup command_id=installed-ui-image-build",
                    completed.stderr,
                )
                self.assertNotIn("reviewed image builder failed safely", completed.stderr)
            if unsafe_build_output:
                self.assertNotIn("cooking-inbound-only-fixture-sentinel", completed.stdout)
                self.assertNotIn("cooking-inbound-only-fixture-sentinel", completed.stderr)
            if expected_status == 0 and scenario == "session-chat":
                self.assertEqual(
                    image_capture.read_text(encoding="utf-8").strip(),
                    explicit_browser_image or "localhost/hephestus-playwright:1.62.0-certutil",
                )
            if expected_status != 0:
                return {}
            if scenario == "session-chat" or browser_e2e:
                self.assertTrue(Path(environment["PREREQUISITE_CAPTURE"]).is_file())
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
                "fork": "1",
                "caddy-wrapper": "1",
                "caddy-port": "41002",
                "installed-ui": "1",
                "cooking": "unset",
            },
        )

    def test_session_chat_reuses_prepared_installed_ui_image_across_phases(self) -> None:
        captured = self.run_wrapper_stub("session-chat", repeat=True)
        self.assertEqual(captured["installed-ui"], "1")

    def test_session_chat_honors_explicit_installed_ui_image_override(self) -> None:
        captured = self.run_wrapper_stub(
            "session-chat",
            explicit_browser_image="registry.example/hephestus-playwright@sha256:" + "c" * 64,
        )
        self.assertEqual(captured["installed-ui"], "1")

    def test_session_chat_loads_reviewed_archive_before_builder(self) -> None:
        captured = self.run_wrapper_stub("session-chat", reviewed_archive=True)
        self.assertEqual(captured["installed-ui"], "1")

    def test_session_chat_rejects_tampered_reviewed_archive_before_load(self) -> None:
        self.run_wrapper_stub(
            "session-chat", reviewed_archive=True, archive_hash_mismatch=True, expected_status=1
        )

    def test_session_chat_retains_safe_image_build_failure(self) -> None:
        self.run_wrapper_stub("session-chat", expected_status=1, build_fail=True)

    def test_session_chat_redacts_unsafe_image_build_output(self) -> None:
        self.run_wrapper_stub(
            "session-chat", expected_status=1, build_fail=True, unsafe_build_output=True
        )

    def test_wrapper_default_cooking_captures_existing_flag(self) -> None:
        captured = self.run_wrapper_stub("cooking")
        self.assertEqual(captured, {"runner": "cooking", "cooking": "1"})

    def test_cooking_browser_prerequisite_runs_without_installed_fixture_mode(self) -> None:
        self.run_wrapper_stub("cooking", browser_e2e=True)

    def test_session_chat_reuses_complete_external_caddy_environment(self) -> None:
        captured = self.run_wrapper_stub("session-chat", complete_caddy=True)
        self.assertEqual(captured["caddy-wrapper"], "unset")
        self.assertEqual(captured["caddy-port"], "42002")
        self.assertEqual(captured["installed-ui"], "1")

    def test_session_chat_rejects_incomplete_marked_caddy_environment(self) -> None:
        captured = self.run_wrapper_stub(
            "session-chat", invalid_complete_marker=True, expected_status=1
        )
        self.assertEqual(captured, {})


if __name__ == "__main__":
    unittest.main()
