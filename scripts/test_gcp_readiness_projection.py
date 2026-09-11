"""Tests for typed custom runner readiness diagnostics."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location(
    "cooking_diagnostics_readiness", ROOT / "collect-cooking-diagnostics.py"
)
assert SPEC is not None and SPEC.loader is not None
COLLECTOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COLLECTOR)


class GcpReadinessProjectionTests(unittest.TestCase):
    def test_all_exact_startup_readiness_errors_project_to_typed_fields(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-readiness-projection-") as raw:
            root = Path(raw)
            for message, (tool, error_class) in COLLECTOR.READINESS_ERRORS.items():
                source = root / f"{tool}-{error_class}.log"
                destination = root / f"{tool}-{error_class}.projected"
                source.write_text(f"gcp-kvm-startup: {message}\n", encoding="utf-8")

                retained, _digest = COLLECTOR._project_text(source, destination)

                self.assertGreater(retained, 0, message)
                self.assertEqual(
                    destination.read_text(encoding="utf-8"),
                    f"HEPH_GCP_RUNNER_IMAGE_READINESS tool={tool} class={error_class} "
                    "phase=runner-image-runtime\n",
                )

    def test_gce_startup_prefix_is_accepted_but_arbitrary_text_is_dropped(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-readiness-prefix-") as raw:
            root = Path(raw)
            source = root / "source.log"
            destination = root / "projected"
            source.write_text(
                "[1700000000.1] google_metadata_script_runner[321]: startup-script: "
                "gcp-kvm-startup: custom runner image ORAS executable cannot run as forge\n"
                "gcp-kvm-startup: arbitrary readiness detail\n",
                encoding="utf-8",
            )

            retained, _digest = COLLECTOR._project_text(source, destination)

            self.assertGreater(retained, 0)
            projected = destination.read_text(encoding="utf-8")
            self.assertIn("tool=oras class=oras-not-runnable", projected)
            self.assertNotIn("arbitrary readiness detail", projected)

    def test_unrecognized_sensitive_assignment_is_rejected_without_projection(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-readiness-sensitive-") as raw:
            root = Path(raw)
            source = root / "source.log"
            destination = root / "projected"
            source.write_text(
                "gcp-kvm-startup: custom runner image Node executable cannot run as forge "
                "token=fixture-value\n",
                encoding="utf-8",
            )

            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR._project_text(source, destination)
            self.assertEqual(destination.read_text(encoding="utf-8"), "")


if __name__ == "__main__":
    unittest.main()
