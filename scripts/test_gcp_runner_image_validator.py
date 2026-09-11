"""Functional tests for the custom immutable-image validator in the smoke script."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest


ROOT = Path(__file__).parent
SMOKE = ROOT / "gcp-kvm-smoke.sh"


class RunnerImageValidatorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        source = SMOKE.read_text(encoding="utf-8")
        marker = 'RUNNER_IMAGE_METADATA="$runner_image_data"'
        start = source.index(marker)
        start = source.index("<<'PY'\n", start) + len("<<'PY'\n")
        cls.validator = source[start:].split("\nPY\n", 1)[0]
        cls.recipe = hashlib.sha256((ROOT / "gcp-runner-image-provision.sh").read_bytes()).hexdigest()
        cls.verifier = hashlib.sha256((ROOT / "gcp-runner-image-verify.py").read_bytes()).hexdigest()
        cls.startup = hashlib.sha256((ROOT / "gcp-kvm-startup.sh").read_bytes()).hexdigest()

    def metadata(self, *, recipe: str | None = None, description_suffix: str = "") -> dict:
        manifest = "a" * 64
        image = "hephaestus-runner-" + manifest[:32]
        description = (
            "hephaestus-runner manifest_sha256=" + manifest
            + " recipe_sha256=" + (recipe or self.recipe)
            + " verifier_sha256=" + self.verifier
            + " startup_sha256=" + self.startup
            + description_suffix
        )
        return {"name": image, "status": "READY", "labels": {
            "purpose": "hephaestus-runner-image",
            "fingerprint": manifest[:32],
            "repository_sha": "c" * 40,
        }, "description": description}

    def run_validator(self, value: dict) -> subprocess.CompletedProcess[str]:
        environment = os.environ | {
            "RUNNER_IMAGE_METADATA": json.dumps(value),
            "RUNNER_IMAGE_RECIPE_SHA": self.recipe,
            "RUNNER_IMAGE_VERIFIER_SHA": self.verifier,
            "RUNNER_IMAGE_STARTUP_SHA": self.startup,
        }
        return subprocess.run(
            [sys.executable, "-", value["name"]], input=self.validator,
            text=True, capture_output=True, env=environment, check=False,
        )

    def test_older_repository_provenance_passes_current_recipe(self) -> None:
        result = self.run_validator(self.metadata())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("runner-image-manifest-sha256=" + "a" * 64, result.stdout)

    def test_changed_recipe_anchor_fails_closed(self) -> None:
        result = self.run_validator(self.metadata(recipe="b" * 64))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match current recipe", result.stderr)

    def test_missing_anchor_fails_closed(self) -> None:
        value = self.metadata()
        value["description"] = value["description"].replace(" startup_sha256=" + self.startup, "")
        result = self.run_validator(value)
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
