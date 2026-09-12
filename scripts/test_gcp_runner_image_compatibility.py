"""Focused tests for the reviewed runner-image runtime compatibility record."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).parent
VALIDATOR = ROOT / "gcp-runner-image-compatibility.py"
RECORDS = ROOT / "gcp-runner-image-compatibility.json"


class RunnerImageCompatibilityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.document = json.loads(RECORDS.read_text(encoding="utf-8"))
        cls.record = cls.document["records"][0]
        cls.recipe = hashlib.sha256((ROOT / "gcp-runner-image-provision.sh").read_bytes()).hexdigest()
        cls.verifier = hashlib.sha256((ROOT / "gcp-runner-image-verify.py").read_bytes()).hexdigest()
        cls.runtime = hashlib.sha256((ROOT / "gcp-kvm-startup.sh").read_bytes()).hexdigest()
        cls.image_name = "hephaestus-runner-" + cls.record["manifest_sha256"][:32]

    def image_metadata(self, **overrides: str) -> dict[str, object]:
        anchors = {
            "manifest_sha256": self.record["manifest_sha256"],
            "recipe_sha256": self.record["baked_recipe_sha256"],
            "verifier_sha256": self.record["baked_verifier_sha256"],
            "startup_sha256": self.record["baked_startup_sha256"],
        }
        anchors.update(overrides)
        description = "hephaestus-runner " + " ".join(f"{key}={value}" for key, value in anchors.items())
        return {
            "name": self.image_name,
            "status": "READY",
            "labels": {
                "purpose": "hephaestus-runner-image",
                "fingerprint": self.record["image_fingerprint"][:32],
                "repository_sha": "c" * 40,
            },
            "description": description,
        }

    def run_controller(
        self,
        metadata: dict[str, object],
        records: Path = RECORDS,
        runtime: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(VALIDATOR),
                "controller",
                "--records-file",
                str(records),
                "--metadata-json",
                json.dumps(metadata),
                "--image-name",
                self.image_name,
                "--expected-recipe",
                self.recipe,
                "--expected-verifier",
                self.verifier,
                "--runtime-startup",
                runtime or self.runtime,
            ],
            text=True,
            capture_output=True,
            check=False,
        )

    def test_exact_reviewed_tuple_is_accepted(self) -> None:
        result = self.run_controller(self.image_metadata())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("runner-image-startup-sha256=" + self.record["baked_startup_sha256"], result.stdout)
        self.assertIn("runner-image-runtime-startup-sha256=" + self.runtime, result.stdout)

    def test_equal_baked_and_runtime_startup_uses_legacy_path(self) -> None:
        result = self.run_controller(self.image_metadata(startup_sha256=self.runtime))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_metadata_name_must_match_requested_image(self) -> None:
        metadata = self.image_metadata()
        metadata["name"] = "hephaestus-runner-other"
        result = self.run_controller(metadata)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match the requested image", result.stderr)

    def test_wrong_baked_anchor_is_rejected(self) -> None:
        result = self.run_controller(self.image_metadata(startup_sha256="a" * 64))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("baked startup", result.stderr)

    def test_missing_image_anchor_is_rejected(self) -> None:
        metadata = self.image_metadata()
        metadata["description"] = str(metadata["description"]).replace(
            " startup_sha256=" + self.record["baked_startup_sha256"], ""
        )
        result = self.run_controller(metadata)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing or duplicated", result.stderr)

    def test_malformed_duplicate_image_anchor_is_rejected(self) -> None:
        metadata = self.image_metadata()
        metadata["description"] = str(metadata["description"]) + " manifest_sha256=malformed"
        result = self.run_controller(metadata)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing or duplicated", result.stderr)

    def test_empty_duplicate_image_anchor_is_rejected(self) -> None:
        metadata = self.image_metadata()
        metadata["description"] = str(metadata["description"]).replace(
            " startup_sha256=" + self.record["baked_startup_sha256"],
            " startup_sha256=,startup_sha256=" + self.record["baked_startup_sha256"],
        )
        result = self.run_controller(metadata)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing or duplicated", result.stderr)

    def test_unknown_runtime_is_rejected(self) -> None:
        result = self.run_controller(self.image_metadata(), runtime="b" * 64)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("runtime startup SHA is not allowed", result.stderr)

    def test_guest_rejects_forged_runtime_anchor(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest_path = Path(directory) / "manifest.json"
            manifest_path.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "kind": "hephaestus-gcp-runner",
                        "manifest_sha256": self.record["manifest_sha256"],
                        "recipe_sha256": self.record["baked_recipe_sha256"],
                        "verifier_sha256": self.record["baked_verifier_sha256"],
                        "startup_sha256": self.record["baked_startup_sha256"],
                    }
                ),
                encoding="utf-8",
            )
            result = subprocess.run(
                [
                    sys.executable,
                    str(VALIDATOR),
                    "guest",
                    "--records-file",
                    str(RECORDS),
                    "--manifest-file",
                    str(manifest_path),
                    "--expected-manifest",
                    self.record["manifest_sha256"],
                    "--expected-recipe",
                    self.record["baked_recipe_sha256"],
                    "--expected-verifier",
                    self.record["baked_verifier_sha256"],
                    "--expected-baked-startup",
                    self.record["baked_startup_sha256"],
                    "--runtime-startup",
                    "b" * 64,
                ],
                text=True,
                capture_output=True,
                check=False,
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("runtime startup SHA is not allowed", result.stderr)

    def test_tampered_staged_record_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            tampered = json.loads(RECORDS.read_text(encoding="utf-8"))
            tampered["records"][0]["allowed_runtime_startup_sha256"] = "b" * 64
            path = Path(directory) / "records.json"
            path.write_text(json.dumps(tampered), encoding="utf-8")
            result = self.run_controller(self.image_metadata(), records=path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("runtime startup SHA is not allowed", result.stderr)

    def test_duplicate_and_unknown_record_fields_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            unknown = json.loads(RECORDS.read_text(encoding="utf-8"))
            unknown["records"][0]["unexpected"] = True
            unknown_path = Path(directory) / "unknown.json"
            unknown_path.write_text(json.dumps(unknown), encoding="utf-8")
            result = self.run_controller(self.image_metadata(), records=unknown_path)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unknown fields", result.stderr)

            duplicate_path = Path(directory) / "duplicate.json"
            raw = RECORDS.read_text(encoding="utf-8").replace(
                '"schema": 1', '"schema": 1, "schema": 1', 1
            )
            duplicate_path.write_text(raw, encoding="utf-8")
            result = self.run_controller(self.image_metadata(), records=duplicate_path)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("duplicate JSON field", result.stderr)

    def test_boolean_schema_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            document = json.loads(RECORDS.read_text(encoding="utf-8"))
            document["schema"] = True
            path = Path(directory) / "boolean-schema.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            result = self.run_controller(self.image_metadata(), records=path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("schema/kind mismatch", result.stderr)


if __name__ == "__main__":
    unittest.main()
