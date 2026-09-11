"""Regression tests for fail-closed disposable-VM cleanup."""

from __future__ import annotations

from pathlib import Path
import os
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
SMOKE = ROOT / "gcp-kvm-smoke.sh"


class GcpKvmSmokeCleanupTests(unittest.TestCase):
    def test_missing_vm_describe_error_does_not_use_unset_json_data(self) -> None:
        run_id = "34599999999"
        name = f"heph-kvm-smoke-{run_id}-1"
        with tempfile.TemporaryDirectory(prefix="heph-gcp-cleanup-") as raw:
            root = Path(raw)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            fake_gcloud = fake_bin / "gcloud"
            fake_gcloud.write_text(
                "#!/usr/bin/env bash\n"
                "set -eu\n"
                "printf \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/"
                f"{name}' was not found\\n\" >&2\n"
                "exit 1\n",
                encoding="utf-8",
            )
            fake_gcloud.chmod(0o700)
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}:{environment['PATH']}",
                    "GITHUB_RUN_ID": run_id,
                    "GITHUB_RUN_ATTEMPT": "1",
                    "GITHUB_SHA": "a" * 40,
                    "GCP_ZONE": "europe-west1-d",
                }
            )
            result = subprocess.run(
                [str(SMOKE), "cleanup"],
                cwd=ROOT.parent,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Disposable VM already absent (verified by describe)", result.stdout)
        self.assertNotIn("unbound variable", result.stderr)

    def test_diagnostic_failure_keeps_typed_marker_outside_recent_serial_tail(self) -> None:
        run_id = "34599999998"
        name = f"heph-kvm-smoke-{run_id}-1"
        secret = "HEPHAESTUS_BROWSER_SECRET_4d7ccf_org"
        serial = "\n".join(
            [
                "HEPH_GCP_KVM_STARTUP event=phase-start phase=diagnostic-bootstrap revision=" + "a" * 40,
                f"fixture token={secret}",
                *[f"ordinary boot line {index}" for index in range(100)],
                "HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS FAIL test_result=expected-failure",
            ]
        )
        with tempfile.TemporaryDirectory(prefix="heph-gcp-serial-context-") as raw:
            root = Path(raw)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            fake_gcloud = fake_bin / "gcloud"
            fake_gcloud.write_text(
                "#!/usr/bin/env bash\n"
                "set -eu\n"
                "case \" $* \" in\n"
                "  *' compute regions describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"INSTANCES\",\"limit\":\"2\",\"usage\":\"0\"}]}' ;;\n"
                "  *' instances describe '*) printf \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/"
                f"{name}' was not found\\n\" >&2; exit 1 ;;\n"
                "  *' instances create '*) exit 0 ;;\n"
                "  *' get-serial-port-output '*) printf '%s\\n' \"${GCP_FAKE_SERIAL}\" ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            fake_gcloud.chmod(0o700)
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}:{environment['PATH']}",
                    "GITHUB_RUN_ID": run_id,
                    "GITHUB_RUN_ATTEMPT": "1",
                    "GITHUB_SHA": "a" * 40,
                    "GCP_ZONE": "europe-west1-d",
                    "GCP_FAKE_SERIAL": serial,
                    "GCP_SMOKE_LOG": str(root / "smoke.log"),
                }
            )
            result = subprocess.run(
                [str(SMOKE), "diagnostic"],
                cwd=ROOT.parent,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Bounded typed serial failure context:", result.stderr)
        self.assertIn("HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS FAIL test_result=expected-failure", result.stderr)
        self.assertIn("HEPH_GCP_KVM_STARTUP event=phase-start phase=diagnostic-bootstrap", result.stderr)
        self.assertNotIn(secret, result.stdout + result.stderr)
        self.assertNotIn("ordinary boot line 99", result.stderr)


if __name__ == "__main__":
    unittest.main()
