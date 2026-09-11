"""Focused local tests for immutable runner-image retirement."""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
SCRIPT = ROOT / "gcp-runner-image-retire.sh"
FINGERPRINT = "a" * 32
IMAGE = "hephaestus-runner-" + FINGERPRINT


FAKE_GCLOUD = r"""#!/usr/bin/env bash
set -Eeuo pipefail
state="${GCP_RETIRE_STATE:?}"
calls="${GCP_RETIRE_CALLS:?}"
printf '%s\n' "$*" >>"$calls"
if [[ "$1 $2 $3" == "compute images describe" ]]; then
  if [[ "${GCP_RETIRE_DESCRIBE_MODE:-}" == permission ]]; then
    printf 'PERMISSION_DENIED: permission denied\n' >&2
    exit 1
  fi
  if [[ -f "$state" ]]; then
    cat "$state"
    exit 0
  fi
  printf "ERROR: The resource 'projects/hephaestus-508000/global/images/%s' was not found.\n" "$4" >&2
  exit 1
fi
if [[ "$1 $2 $3" == "compute images delete" ]]; then
  if [[ "${GCP_RETIRE_DELETE_MODE:-}" == permission ]]; then
    printf 'PERMISSION_DENIED: permission denied\n' >&2
    exit 1
  fi
  rm -f -- "$state"
  exit 0
fi
printf 'unexpected gcloud operation: %s\n' "$*" >&2
exit 2
"""


class RunnerImageRetireTests(unittest.TestCase):
    def environment(self, directory: Path, metadata: dict | None = None) -> dict[str, str]:
        fake_bin = directory / "bin"
        fake_bin.mkdir()
        fake_gcloud = fake_bin / "gcloud"
        fake_gcloud.write_text(FAKE_GCLOUD, encoding="utf-8")
        fake_gcloud.chmod(0o700)
        state = directory / "image.json"
        if metadata is not None:
            state.write_text(json.dumps(metadata), encoding="utf-8")
        return os.environ | {
            "PATH": f"{fake_bin}:{os.environ['PATH']}",
            "GCP_RETIRE_STATE": str(state),
            "GCP_RETIRE_CALLS": str(directory / "calls"),
        }

    @staticmethod
    def metadata(*, purpose: str = "hephaestus-runner-image") -> dict:
        manifest = "a" * 64
        return {
            "name": IMAGE,
            "status": "READY",
            "selfLink": f"https://compute.googleapis.com/compute/v1/projects/hephaestus-508000/global/images/{IMAGE}",
            "labels": {
                "purpose": purpose,
                "fingerprint": FINGERPRINT,
                "repository_sha": "b" * 40,
            },
            "description": (
                f"manifest_sha256={manifest} "
                f"recipe_sha256={'c' * 64} "
                f"verifier_sha256={'d' * 64} "
                f"startup_sha256={'e' * 64}"
            ),
        }

    def invoke(self, directory: Path, environment: dict[str, str]) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(SCRIPT), "retire", IMAGE],
            env=environment,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_success_deletes_only_candidate_and_verifies_absence(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-image-retire-") as raw:
            directory = Path(raw)
            result = self.invoke(directory, self.environment(directory, self.metadata()))
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("retirement verified image absent", result.stdout)
            calls = (directory / "calls").read_text(encoding="utf-8").splitlines()
            self.assertEqual(sum("compute images delete" in call for call in calls), 1)
            self.assertEqual(sum("compute images describe" in call for call in calls), 2)
            self.assertTrue(all("disks" not in call and "instances" not in call for call in calls))

    def test_current_image_is_protected_before_provider_call(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-image-retire-protected-") as raw:
            directory = Path(raw)
            environment = self.environment(directory, self.metadata())
            environment["GCP_PROTECTED_CURRENT_IMAGE"] = IMAGE
            result = self.invoke(directory, environment)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("configured current image", result.stderr)
            self.assertFalse((directory / "calls").exists())

    def test_unowned_purpose_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-image-retire-unowned-") as raw:
            directory = Path(raw)
            result = self.invoke(directory, self.environment(directory, self.metadata(purpose="other")))
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("immutable provenance validation", result.stderr)
            self.assertFalse(any("compute images delete" in call for call in (directory / "calls").read_text(encoding="utf-8").splitlines()))

    def test_malformed_name_is_rejected_without_provider_call(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-image-retire-malformed-") as raw:
            directory = Path(raw)
            environment = self.environment(directory)
            result = subprocess.run(
                [str(SCRIPT), "retire", "hephaestus-runner-not-a-fingerprint"],
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("runner image name is invalid", result.stderr)
            self.assertFalse((directory / "calls").exists())

    def test_permission_failure_fails_closed_before_delete(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-image-retire-permission-") as raw:
            directory = Path(raw)
            environment = self.environment(directory, self.metadata())
            environment["GCP_RETIRE_DESCRIBE_MODE"] = "permission"
            result = self.invoke(directory, environment)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("cannot describe candidate image", result.stderr)
            calls = (directory / "calls").read_text(encoding="utf-8").splitlines()
            self.assertFalse(any("compute images delete" in call for call in calls))

    def test_delete_permission_failure_fails_if_candidate_remains(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-image-retire-delete-permission-") as raw:
            directory = Path(raw)
            environment = self.environment(directory, self.metadata())
            environment["GCP_RETIRE_DELETE_MODE"] = "permission"
            result = self.invoke(directory, environment)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("was not verified absent", result.stderr)
            calls = (directory / "calls").read_text(encoding="utf-8").splitlines()
            self.assertEqual(sum("compute images delete" in call for call in calls), 1)


if __name__ == "__main__":
    unittest.main()
