"""Contract checks for the opt-in session-chat negative-process runner."""

from pathlib import Path
import os
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent
SCRIPT = ROOT / "run-libkrun-integration.sh"
GCP_RUNNER = ROOT / "gcp-cooking-run.sh"
SELECTED_TEST = "bearer_push_starts_run_through_production_bootstrap"


class LibkrunNegativeOrchestrationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.script = SCRIPT.read_text(encoding="utf-8")
        cls.gcp_runner = GCP_RUNNER.read_text(encoding="utf-8")

    def negative_block(self) -> str:
        first_end = self.script.index("phase_timing_end golden-tests passed")
        start = self.script.index(
            'if [[ "${HEPHAESTUS_APP_SESSION_CHAT_NEGATIVE_E2E:-0}" == "1" ]]; then',
            first_end,
        )
        end = self.script.index(
            "\n    # Reuse the same disposable authority database", start
        )
        return self.script[start:end]

    def test_shell_script_parses(self) -> None:
        result = subprocess.run(
            ["bash", "-n", str(SCRIPT)], capture_output=True, text=True, check=False
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_negative_mode_requires_the_complete_canonical_journey(self) -> None:
        gate = self.script[self.script.index("if [[ \"${HEPHAESTUS_APP_LIBKRUN_E2E") :]
        gate = gate[: gate.index("printf 'Running daemon golden E2E")]
        self.assertIn(
            '[[ "${HEPHAESTUS_APP_SESSION_CHAT_NEGATIVE_E2E:-0}" == "1" ]]',
            gate,
        )
        self.assertIn(
            '[[ "${HEPHAESTUS_APP_SESSION_CHAT_E2E:-0}" == "1" ]]',
            gate,
        )
        for required_flag in (
            "HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E",
            "HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E",
            "HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E",
            "HEPHAESTUS_APP_SESSION_CHAT_FORK_E2E",
        ):
            self.assertIn(required_flag, gate)

    def run_extracted_negative_block(self, cargo_status: int):
        block = self.negative_block()
        with tempfile.TemporaryDirectory(prefix="libkrun-negative-contract-") as directory:
            root = Path(directory)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            capture_env = root / "cargo-env"
            capture_args = root / "cargo-args"
            timing = root / "timing"
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                f"env >{capture_env!s}\n"
                f": >{capture_args!s}\n"
                f"printf '%s\\n' \"$@\" >>{capture_args!s}\n"
                f"exit \"${{FAKE_CARGO_STATUS:-{cargo_status}}}\"\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o700)
            harness = root / "harness.sh"
            harness.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "phase_timing_open=''\n"
                "phase_timing_start() {\n"
                "  printf 'start %s\\n' \"$1\" >>\"$TIMING_CAPTURE\"\n"
                "  phase_timing_open=$1\n"
                "}\n"
                "phase_timing_end() {\n"
                "  printf 'end %s %s\\n' \"$1\" \"$2\" >>\"$TIMING_CAPTURE\"\n"
                "  phase_timing_open=''\n"
                "}\n"
                "phase_timing_finish_open() {\n"
                "  [[ -n \"$phase_timing_open\" ]] || return 0\n"
                "  phase_timing_end \"$phase_timing_open\" failed\n"
                "}\n"
                "cleanup() {\n"
                "  local status=$?\n"
                "  trap - EXIT\n"
                "  phase_timing_finish_open \"$status\"\n"
                "  exit \"$status\"\n"
                "}\n"
                "trap cleanup EXIT\n"
                "run_as_guest_owner() { \"$@\"; }\n"
                f"TIMING_CAPTURE={timing!s}\n"
                f"PATH={fake_bin!s}:$PATH\n"
                "HEPHAESTUS_APP_SESSION_CHAT_NEGATIVE_E2E=1\n"
                "HEPHAESTUS_APP_SESSION_CHAT_E2E=1\n"
                "HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E=1\n"
                "HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E=1\n"
                "HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E=1\n"
                "HEPHAESTUS_APP_SESSION_CHAT_FORK_E2E=1\n"
                "HEPHAESTUS_COOKING_BROWSER_E2E=1\n"
                "HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE=1\n"
                "HEPH_GCP_PHASE_TIMING_PATH=/private/timing.jsonl\n"
                "HEPH_GCP_PHASE_TIMING_SOURCE_SHA=source\n"
                "HEPH_GCP_PHASE_TIMING_RUN_ID=run\n"
                "HEPH_GCP_PHASE_TIMING_ATTEMPT=attempt\n"
                "HEPH_GCP_PHASE_TIMING_IMAGE_FINGERPRINT=image\n"
                "HEPHAESTUS_GIT_PRE_RECEIVE_HOOK=/protected/pre-receive\n"
                "export HEPHAESTUS_APP_SESSION_CHAT_NEGATIVE_E2E HEPHAESTUS_APP_SESSION_CHAT_E2E\n"
                "export HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E\n"
                "export HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E HEPHAESTUS_APP_SESSION_CHAT_FORK_E2E\n"
                "export HEPHAESTUS_COOKING_BROWSER_E2E HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE\n"
                "export HEPH_GCP_PHASE_TIMING_PATH HEPH_GCP_PHASE_TIMING_SOURCE_SHA HEPH_GCP_PHASE_TIMING_RUN_ID\n"
                "export HEPH_GCP_PHASE_TIMING_ATTEMPT HEPH_GCP_PHASE_TIMING_IMAGE_FINGERPRINT\n"
                "export HEPHAESTUS_GIT_PRE_RECEIVE_HOOK\n"
                "fixture_root=/fixture\n"
                "postgres_url=postgres://fixture\n"
                "nats_url=nats://fixture\n"
                "cgroup_root=/cgroup\n"
                "repo_root=/repo\n"
                "rust_builder_root=/rust-builder\n"
                "cargo_target_dir=/target\n"
                "GUEST_TARGET=x86_64-unknown-linux-musl\n"
                "target_directory=/target\n"
                "builder_vm_image=builder\n"
                "verifier_vm_image=verifier\n"
                "base_layout_manifest=/manifest\n"
                "builder_layout=/builder-layout\n"
                "verifier_layout=/verifier-layout\n"
                "builder_operation_root=/builder-root\n"
                "verifier_operation_root=/verifier-root\n"
                "golden_features=()\n"
                f"FAKE_CARGO_STATUS={cargo_status}\n"
                f"{block}\n",
                encoding="utf-8",
            )
            harness.chmod(0o700)
            completed = subprocess.run(
                ["bash", str(harness)],
                capture_output=True,
                text=True,
                check=False,
                env={"PATH": os.environ["PATH"]},
            )
            if not capture_env.exists():
                raise AssertionError(f"harness failed before fake cargo: {completed.stderr}")
            env_values = {}
            for line in capture_env.read_text(encoding="utf-8").splitlines():
                key, separator, value = line.partition("=")
                if separator:
                    env_values[key] = value
            args = capture_args.read_text(encoding="utf-8").splitlines()
            phases = timing.read_text(encoding="utf-8").splitlines()
            return completed, env_values, args, phases

    def test_second_process_is_exactly_one_fresh_denial_test(self) -> None:
        completed, values, args, phases = self.run_extracted_negative_block(0)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(
            args,
            [
                "test",
                "--manifest-path",
                "/repo/Cargo.toml",
                "--package",
                "hephaestus-app",
                "--test",
                "golden",
                SELECTED_TEST,
                "--",
                "--exact",
                "--nocapture",
            ],
        )
        self.assertEqual(values["HEPHAESTUS_APP_SESSION_CHAT_DENIAL_PROBE_E2E"], "1")
        self.assertEqual(values["HEPHAESTUS_APP_SESSION_CHAT_NEGATIVE_E2E"], "1")
        self.assertEqual(values["HEPHAESTUS_APP_SESSION_CHAT_E2E"], "1")
        self.assertEqual(values["HEPHAESTUS_GIT_PRE_RECEIVE_HOOK"], "/protected/pre-receive")
        self.assertEqual(values["HEPHAESTUS_LIBKRUN_RUNTIME_ROOT"], "/fixture/runtime")
        for absent in (
            "HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E",
            "HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E",
            "HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E",
            "HEPHAESTUS_APP_SESSION_CHAT_FORK_E2E",
            "HEPHAESTUS_COOKING_BROWSER_E2E",
            "HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE",
            "HEPH_GCP_PHASE_TIMING_PATH",
            "HEPH_GCP_PHASE_TIMING_SOURCE_SHA",
            "HEPH_GCP_PHASE_TIMING_RUN_ID",
            "HEPH_GCP_PHASE_TIMING_ATTEMPT",
            "HEPH_GCP_PHASE_TIMING_IMAGE_FINGERPRINT",
        ):
            self.assertNotIn(absent, values)
        self.assertEqual(
            phases,
            ["start guest-negative-capability", "end guest-negative-capability passed"],
        )

    def test_second_process_failure_propagates_and_closes_outer_phase(self) -> None:
        completed, _, _, phases = self.run_extracted_negative_block(17)
        self.assertEqual(completed.returncode, 17)
        self.assertEqual(
            phases,
            ["start guest-negative-capability", "end guest-negative-capability failed"],
        )

    def test_gcp_does_not_enable_opt_in_or_capture_extra_stdout(self) -> None:
        self.assertNotIn("HEPHAESTUS_APP_SESSION_CHAT_NEGATIVE_E2E=1", self.gcp_runner)
        block = self.negative_block()
        self.assertNotIn("tee", block)
        self.assertIn("-- --exact --nocapture", block)
        self.assertIn('phase_timing_finish_open "${status}"', self.script)


if __name__ == "__main__":
    unittest.main()
