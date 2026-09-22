"""Functional tests for bounded, credential-safe runner serial diagnostics."""
from __future__ import annotations

from pathlib import Path
import json
import subprocess
import tempfile
import time
import unittest

try:
    from . import test_gcp_runner_image_build as image_build_tests
except ImportError:  # pragma: no cover - supports direct unittest discovery
    import test_gcp_runner_image_build as image_build_tests


ROOT = Path(__file__).parent
BUILD = ROOT / "gcp-runner-image-build.sh"
REVISION = "a" * 40


def fake_gcloud_with_serial_output() -> str:
    needle = 'if [[ "${GCP_FAKE_SERIAL:-ready}" == fail ]]; then\n'
    replacement = (
        'if [[ -n "${GCP_FAKE_SERIAL_OUTPUT:-}" ]]; then\n'
        '      printf "%s\\n" "$GCP_FAKE_SERIAL_OUTPUT"\n'
        '    elif [[ "${GCP_FAKE_SERIAL:-ready}" == fail ]]; then\n'
    )
    return image_build_tests.FAKE_GCLOUD.replace(needle, replacement)


class RunnerImageSerialTests(unittest.TestCase):
    def _run(self, root: Path, serial: str) -> tuple[subprocess.CompletedProcess[str], Path]:
        setup = image_build_tests.RunnerImageBuildTests()
        environment = setup._setup(root) | {
            "GCP_RUNNER_IMAGE_SERIAL_LOG": str(root / "serial.log"),
            "GCP_FAKE_SERIAL_OUTPUT": serial,
        }
        fake = root / "bin" / "gcloud"
        fake.write_text(fake_gcloud_with_serial_output(), encoding="utf-8")
        result = subprocess.run([str(BUILD), "build"], env=environment,
                                text=True, capture_output=True, check=False)
        return result, Path(environment["GCP_RUNNER_IMAGE_SERIAL_LOG"])

    def test_failure_persists_only_safe_latest_serial_projection(self) -> None:
        serial = (
            f"[ 1.0] google_metadata_script_runner[123]: startup-script: "
            f"HEPH_GCP_KVM_STARTUP event=phase-start phase=accounts revision={REVISION}\n"
            f"[ 1.1] google_metadata_script_runner[123]: startup-script: "
            f"HEPH_GCP_KVM_STARTUP event=phase-start phase=accounts revision={REVISION}\n"
            "HEPH_GCP_RUNNER_IMAGE: FAIL exit=17"
        )
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-") as raw:
            result, serial_log = self._run(Path(raw), serial)
            self.assertNotEqual(result.returncode, 0)
            content = serial_log.read_text(encoding="utf-8")
            self.assertIn(f"phase=accounts revision={REVISION}", content)
            self.assertIn("terminal=fail exit=17", content)
            tail = Path(f"{serial_log}.tail")
            self.assertTrue(tail.is_file())
            self.assertIn("HEPH_GCP_RUNNER_IMAGE: FAIL exit=17", tail.read_text(encoding="utf-8"))
            self.assertIn("Bounded runner serial phase diagnostics:", result.stderr)
            self.assertIn("Bounded runner serial error tail:", result.stderr)
            self.assertEqual(result.stdout.count("phase=accounts"), 1)

    def test_earliest_typed_build_failure_is_projected_with_closed_command_id(self) -> None:
        serial = (
            f"HEPH_GCP_KVM_STARTUP event=phase-start phase=libkrun revision={REVISION}\n"
            "HEPH_GCP_KVM_BUILD_ERROR phase=libkrun status=23 log=/srv/hephaestus/libkrun.log\n"
            "HEPH_GCP_KVM_FIRST_ERROR make: *** [target] Error 23\n"
            "HEPH_GCP_KVM_BUILD_ERROR phase=rust-toolchain status=24 log=/srv/hephaestus/rust.log\n"
            "HEPH_GCP_RUNNER_IMAGE: FAIL exit=23\n"
        )
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-first-") as raw:
            root = Path(raw)
            result, serial_log = self._run(root, serial)
            self.assertNotEqual(result.returncode, 0)
            failure_path = root / "serial.log.first-failure.json"
            failure = json.loads(failure_path.read_text(encoding="utf-8"))
            self.assertEqual(
                failure,
                {
                    "command_id": "image-bake-libkrun",
                    "diagnostic_error": "image-build-failed",
                    "diagnostic_source": "scanned-serial",
                    "exit_code": 23,
                    "phase": "libkrun",
                },
            )
            self.assertIn("failure phase=libkrun command=image-bake-libkrun exit=23", serial_log.read_text(encoding="utf-8"))
            self.assertNotIn("rust-toolchain", (root / "serial.log.first-failure.json").read_text(encoding="utf-8"))

    def test_wrapper_failure_without_typed_marker_is_not_attributed_to_a_phase(self) -> None:
        serial = "HEPH_GCP_RUNNER_IMAGE: FAIL exit=17\n"
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-wrapper-") as raw:
            root = Path(raw)
            result, _ = self._run(root, serial)
            self.assertNotEqual(result.returncode, 0)
            failure = json.loads((root / "serial.log.first-failure.json").read_text(encoding="utf-8"))
            self.assertEqual(failure["phase"], "unknown")
            self.assertEqual(failure["command_id"], "runner-image-bake-wrapper")
            self.assertEqual(failure["diagnostic_error"], "wrapped-terminal-failure")
            self.assertEqual(failure["exit_code"], 17)

    def test_typed_installed_ui_build_failure_is_reported_as_browser_setup(self) -> None:
        serial = (
            f"[ 4.0] google_metadata_script_runner[123]: startup-script: HEPH_GCP_KVM_STARTUP event=phase-start phase=image-bake-ready revision={REVISION}\n"
            "HEPH_GCP_COOKING event=installed-ui-image status=build-failed "
            "exit_code=17 scan_exit_code=0 log_exit_code=0\n"
            "HEPH_GCP_KVM_FIRST_ERROR podman build failed\n"
            "HEPH_GCP_RUNNER_IMAGE: FAIL exit=101\n"
        )
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-browser-") as raw:
            root = Path(raw)
            result, serial_log = self._run(root, serial)
            self.assertNotEqual(result.returncode, 0)
            failure_path = root / "serial.log.first-failure.json"
            failure = json.loads(failure_path.read_text(encoding="utf-8"))
            self.assertEqual(failure["phase"], "browser-setup")
            self.assertEqual(failure["command_id"], "installed-ui-image-build")
            self.assertEqual(failure["exit_code"], 17)
            self.assertIn("command=installed-ui-image-build", serial_log.read_text(encoding="utf-8"))
            self.assertEqual(failure_path.stat().st_mode & 0o777, 0o600)

    def test_zero_exit_typed_build_failure_is_ignored(self) -> None:
        serial = (
            "HEPH_GCP_KVM_BUILD_ERROR phase=libkrun status=0 log=/srv/hephaestus/libkrun.log\n"
            "HEPH_GCP_RUNNER_IMAGE: FAIL exit=17\n"
        )
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-invalid-") as raw:
            root = Path(raw)
            result, _ = self._run(root, serial)
            self.assertNotEqual(result.returncode, 0)
            failure = json.loads((root / "serial.log.first-failure.json").read_text(encoding="utf-8"))
            self.assertEqual(failure["phase"], "unknown")
            self.assertEqual(failure["command_id"], "runner-image-bake-wrapper")
            self.assertEqual(failure["exit_code"], 17)

    def test_typed_failure_survives_credential_rejection_without_secret(self) -> None:
        secret = "fixture-secret-value"
        serial = (
            "HEPH_GCP_KVM_BUILD_ERROR phase=libkrun status=23 log=/srv/hephaestus/libkrun.log\n"
            f"HEPH_GCP_KVM_FIRST_ERROR token={secret}\n"
            "HEPH_GCP_RUNNER_IMAGE: FAIL exit=23\n"
        )
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-first-secret-") as raw:
            root = Path(raw)
            result, serial_log = self._run(root, serial)
            self.assertNotEqual(result.returncode, 0)
            failure_text = (root / "serial.log.first-failure.json").read_text(encoding="utf-8")
            self.assertIn('"phase":"libkrun"', failure_text)
            self.assertNotIn(secret, failure_text)
            self.assertNotIn(secret, serial_log.read_text(encoding="utf-8"))
            self.assertIn("serial_diagnostics=rejected reason=credential-scan", serial_log.read_text(encoding="utf-8"))

    def test_safe_builder_error_tail_is_retained_and_bounded(self) -> None:
        serial = (
            f"[ 1.0] google_metadata_script_runner[123]: startup-script: "
            f"HEPH_GCP_KVM_STARTUP event=phase-start phase=libkrun revision={REVISION}\n"
            "npm ERR! code EACCES\n"
            "make: *** [target] Error 2\n"
            "HEPH_GCP_RUNNER_IMAGE: FAIL exit=17"
        )
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-errors-") as raw:
            result, serial_log = self._run(Path(raw), serial)
            self.assertNotEqual(result.returncode, 0)
            tail = Path(f"{serial_log}.tail")
            content = tail.read_text(encoding="utf-8")
            self.assertIn("npm ERR! code EACCES", content)
            self.assertIn("make: *** [target] Error 2", content)
            self.assertLessEqual(tail.stat().st_size, 64 * 1024)
            self.assertIn(f"phase=libkrun revision={REVISION}", serial_log.read_text(encoding="utf-8"))

    def test_credential_in_serial_is_rejected_without_leaking_value(self) -> None:
        secret = "fixture-secret-value"
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-secret-") as raw:
            result, serial_log = self._run(
                Path(raw),
                f"HEPH_GCP_KVM_STARTUP event=phase-start phase=accounts revision={REVISION}\n"
                f"fixture access_token={secret}\n"
                "HEPH_GCP_RUNNER_IMAGE: FAIL exit=17",
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("serial diagnostics rejected by credential scanner", result.stderr)
            self.assertNotIn(secret, result.stdout + result.stderr)
            content = serial_log.read_text(encoding="utf-8")
            self.assertNotIn(secret, content)
            self.assertIn("serial_diagnostics=rejected reason=credential-scan", content)
            tail = Path(f"{serial_log}.tail")
            self.assertNotIn(secret, tail.read_text(encoding="utf-8"))

    def test_credential_line_does_not_block_later_ready_marker(self) -> None:
        secret = "fixture-secret-value"
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-ready-") as raw:
            setup = image_build_tests.RunnerImageBuildTests()
            environment = setup._setup(Path(raw)) | {
                "GCP_RUNNER_IMAGE_SERIAL_LOG": str(Path(raw) / "serial.log"),
                "GCP_FAKE_SERIAL_OUTPUT": (
                    f"fixture token={secret}\n"
                    "HEPH_GCP_RUNNER_IMAGE: READY fingerprint=" + image_build_tests.FINGERPRINT
                ),
            }
            fake = Path(raw) / "bin" / "gcloud"
            fake.write_text(fake_gcloud_with_serial_output(), encoding="utf-8")
            result = subprocess.run([str(BUILD), "build"], env=environment,
                                    text=True, capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("serial diagnostics rejected by credential scanner", result.stderr)
            self.assertNotIn(secret, result.stdout + result.stderr)

    def test_multiline_private_key_tail_is_quarantined_but_phase_is_kept(self) -> None:
        key_body = "fixture-private-key-body-must-not-appear"
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-pem-") as raw:
            result, serial_log = self._run(
                Path(raw),
                f"HEPH_GCP_KVM_STARTUP event=phase-start phase=libkrun revision={REVISION}\n"
                "-----BEGIN PRIVATE KEY-----\n"
                f"{key_body}\n"
                "-----END PRIVATE KEY-----\n"
                "HEPH_GCP_RUNNER_IMAGE: READY fingerprint=" + image_build_tests.FINGERPRINT,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            tail = Path(f"{serial_log}.tail")
            self.assertEqual(tail.read_text(encoding="utf-8"), "")
            self.assertIn(f"phase=libkrun revision={REVISION}", serial_log.read_text(encoding="utf-8"))
            self.assertNotIn(key_body, serial_log.read_text(encoding="utf-8"))
            self.assertNotIn(key_body, result.stdout + result.stderr)

    def test_naked_known_fixture_value_quarantines_whole_tail(self) -> None:
        fixture = "HEPHAESTUS_BROWSER_SECRET_4d7ccf_org"
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-known-") as raw:
            result, serial_log = self._run(
                Path(raw),
                f"HEPH_GCP_KVM_STARTUP event=phase-start phase=accounts revision={REVISION}\n"
                f"{fixture}\n"
                "HEPH_GCP_RUNNER_IMAGE: READY fingerprint=" + image_build_tests.FINGERPRINT,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            tail = Path(f"{serial_log}.tail")
            self.assertEqual(tail.read_text(encoding="utf-8"), "")
            self.assertNotIn(fixture, serial_log.read_text(encoding="utf-8"))
            self.assertIn("serial_diagnostics=rejected reason=credential-scan", serial_log.read_text(encoding="utf-8"))

    def test_gce_prefix_and_crlf_failure_returns_without_poll_timeout(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-prefix-") as raw:
            started = time.monotonic()
            result, serial_log = self._run(
                Path(raw),
                f"[1700000000.1] google_metadata_script_runner[321]: startup-script: "
                f"HEPH_GCP_KVM_STARTUP event=phase-start phase=accounts revision={REVISION}\r\n"
                "[1700000000.2] google_metadata_script_runner[321]: startup-script: "
                "HEPH_GCP_RUNNER_IMAGE: FAIL exit=17\r\n",
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertLess(time.monotonic() - started, 2)
            self.assertIn("terminal=fail exit=17", serial_log.read_text(encoding="utf-8"))

    def test_cleanup_does_not_truncate_persisted_serial_tail(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-serial-cleanup-") as raw:
            root = Path(raw)
            setup = image_build_tests.RunnerImageBuildTests()
            environment = setup._setup(root) | {
                "GCP_RUNNER_IMAGE_SERIAL_LOG": str(root / "serial.log"),
            }
            serial_log = Path(environment["GCP_RUNNER_IMAGE_SERIAL_LOG"])
            serial_tail = Path(f"{serial_log}.tail")
            serial_log.write_text("HEPH_GCP_IMAGE_BUILD phase=accounts revision=" + REVISION + "\n", encoding="utf-8")
            serial_tail.write_text("make: *** [target] Error 2\n", encoding="utf-8")
            result = subprocess.run([str(BUILD), "cleanup"], env=environment,
                                    text=True, capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(serial_tail.read_text(encoding="utf-8"), "make: *** [target] Error 2\n")
            self.assertIn("make: *** [target] Error 2", result.stderr)


if __name__ == "__main__":
    unittest.main()
