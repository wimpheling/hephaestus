"""Contract checks for controller-side runner-image compatibility wiring."""
from __future__ import annotations

from pathlib import Path
import unittest


ROOT = Path(__file__).parent


class RunnerImageValidatorWiringTests(unittest.TestCase):
    def test_controller_uses_strict_compatibility_validator_and_stages_both_files(self) -> None:
        source = (ROOT / "gcp-kvm-smoke.sh").read_text(encoding="utf-8")
        self.assertIn("gcp-runner-image-compatibility.py", source)
        self.assertIn("gcp-runner-image-compatibility.json", source)
        self.assertIn("--runtime-startup", source)
        self.assertIn("--runtime-startup", source)
        self.assertIn("runner-image-compatibility-validator-sha256", source)
        self.assertIn("runner-image-compatibility-records-sha256", source)

    def test_guest_hashes_the_executing_runtime_script(self) -> None:
        source = (ROOT / "gcp-kvm-startup.sh").read_text(encoding="utf-8")
        self.assertIn('sha256sum "$runtime_startup_script_path"', source)
        self.assertIn("executing runtime startup script does not match", source)
        self.assertIn("runner image failed reviewed runtime compatibility validation", source)


if __name__ == "__main__":
    unittest.main()
