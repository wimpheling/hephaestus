"""Focused tests for same-repository PR provenance validation."""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
SCRIPT = ROOT / "verify-gcp-pr-provenance.py"
REVISION = "a" * 40


def response(*, sha: str = REVISION, head_id: int = 1312377552, state: str = "open") -> dict:
    repository = {"id": 1312377552, "full_name": "wimpheling/hephaestus"}
    return {
        "number": 42,
        "state": state,
        "head": {"sha": sha, "repo": {**repository, "id": head_id}},
        "base": {"repo": repository},
    }


class ProvenanceTests(unittest.TestCase):
    def run_validator(self, document: dict, *, sha: str = REVISION, number: str = "42", repository_id: str = "1312377552") -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory(prefix="heph-pr-provenance-") as directory:
            path = Path(directory) / "response.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            return subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--response-file",
                    str(path),
                    "--pull-number",
                    number,
                    "--repository-id",
                    repository_id,
                    "--head-sha",
                    sha,
                ],
                text=True,
                capture_output=True,
                check=False,
            )

    def test_accepts_current_open_same_repository_head(self) -> None:
        result = self.run_validator(response())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"head_sha={REVISION}", result.stdout)

    def test_rejects_sha_drift_and_forks(self) -> None:
        for document, kwargs in (
            (response(sha="b" * 40), {}),
            (response(head_id=999), {}),
            (response(), {"repository_id": "999"}),
            (response(state="closed"), {}),
            (response(), {"number": "0"}),
        ):
            with self.subTest(document=document, kwargs=kwargs):
                result = self.run_validator(document, **kwargs)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("PR provenance rejected:", result.stderr)

    def test_rejects_malformed_response_and_head_sha(self) -> None:
        result = self.run_validator({"number": 42}, sha="not-a-sha")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("head SHA is invalid", result.stderr)


if __name__ == "__main__":
    unittest.main()
