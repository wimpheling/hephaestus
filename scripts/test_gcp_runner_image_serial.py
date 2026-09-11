"""Functional tests for bounded, credential-safe runner serial diagnostics."""
from __future__ import annotations

from pathlib import Path
import subprocess
import tempfile
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


if __name__ == "__main__":
    unittest.main()
