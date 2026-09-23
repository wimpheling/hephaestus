"""Focused projector-to-collector-to-triage tests for the negative sidecar."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import stat
import tempfile
import unittest


ROOT = Path(__file__).parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / filename)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


COLLECTOR = _load("negative_pipeline_collector", "collect-cooking-diagnostics.py")
TRIAGE = _load("negative_pipeline_triage", "summarize-cooking-diagnostics.py")
PROJECTOR = _load("negative_pipeline_projector", "project-session-chat-negative-summary.py")


class SessionChatNegativeSummaryPipelineTests(unittest.TestCase):
    def _log(self, root: Path) -> Path:
        path = root / "private-process.log"
        path.write_text(
            "running 1 test\n"
            "test bearer_push_starts_run_through_production_bootstrap ... ok\n"
            "HEPH_SESSION_CHAT_DENIAL_PROBE host=validated checks=10 refs=unchanged receives=unchanged\n"
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n",
            encoding="utf-8",
        )
        return path

    def _collect(self, root: Path, sidecar: Path | None, *, missing: bool = False) -> dict:
        root.mkdir(parents=True, exist_ok=True)
        serial = root / "serial.log"
        serial.write_text("HEPH_GCP_KVM_STARTUP event=phase-pass phase=running\n", encoding="utf-8")
        sources = [f"serial={serial}"]
        if sidecar is not None:
            sources.append(f"session-chat-negative-summary={sidecar}")
        elif missing:
            sources.append(f"session-chat-negative-summary={root / 'missing.json'}")
        bundle = root / "bundle"
        self.assertEqual(COLLECTOR.collect(bundle, sources, None, None, None), 0)
        return TRIAGE.summarize(bundle)

    def test_valid_and_runner_failure_flow_to_safe_statuses(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            log = self._log(root)
            sidecar = root / "negative-summary.json"
            PROJECTOR.write_atomic(sidecar, PROJECTOR.project(log, 0))
            summary = self._collect(root, sidecar)
            self.assertEqual(
                summary["sessionChatNegativeSummary"],
                {"status": "passed", "reason": "validated"},
            )
            self.assertEqual(stat.S_IMODE(sidecar.stat().st_mode), 0o600)
            self.assertEqual(
                stat.S_IMODE((root / "bundle/sources/session-chat-negative-summary").stat().st_mode),
                0o600,
            )
            failed_sidecar = root / "negative-summary-failed.json"
            PROJECTOR.write_atomic(failed_sidecar, PROJECTOR.project(log, 7))
            failed = self._collect(root / "failed", failed_sidecar)
            self.assertEqual(
                failed["sessionChatNegativeSummary"],
                {"status": "failed", "reason": "runner_nonzero"},
            )

    def test_missing_and_malformed_sidecars_are_unknown_without_raw_fields(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            missing = self._collect(root / "missing", None, missing=True)
            self.assertEqual(
                missing["sessionChatNegativeSummary"],
                {"status": "unknown", "reason": "missing"},
            )
            secret = "PRIVATE_NEGATIVE_SUMMARY_SECRET"
            malformed = root / "malformed.json"
            malformed.parent.mkdir(parents=True, exist_ok=True)
            malformed.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "scenario": "session-chat-negative-capability",
                        "status": "passed",
                        "reason": "validated",
                        "validated_checks": 10,
                        "refs": "unchanged",
                        "receives": "unchanged",
                        "golden_test": "bearer_push_starts_run_through_production_bootstrap",
                        "golden_test_passes": 1,
                        "runner_exit_status": 0,
                        "raw": secret,
                    }
                ),
                encoding="utf-8",
            )
            malformed_summary = self._collect(root / "malformed", malformed)
            self.assertEqual(
                malformed_summary["sessionChatNegativeSummary"],
                {"status": "unknown", "reason": "rejected"},
            )
            encoded = json.dumps(malformed_summary, sort_keys=True)
            self.assertNotIn(secret, encoded)

    def test_rejects_noncanonical_schema_keys_and_duplicate_json_fields(self):
        valid = {
            "schema": 1,
            "scenario": "session-chat-negative-capability",
            "status": "passed",
            "reason": "validated",
            "validated_checks": 10,
            "refs": "unchanged",
            "receives": "unchanged",
            "golden_test": "bearer_push_starts_run_through_production_bootstrap",
            "golden_test_passes": 1,
            "runner_exit_status": 0,
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cases = {
                "bool-schema": {**valid, "schema": True},
                "missing-common": {key: value for key, value in valid.items() if key != "reason"} | {"extra": "ignored"},
            }
            for name, value in cases.items():
                sidecar = root / f"{name}.json"
                sidecar.write_text(json.dumps(value), encoding="utf-8")
                summary = self._collect(root / name, sidecar)
                self.assertEqual(
                    summary["sessionChatNegativeSummary"],
                    {"status": "unknown", "reason": "rejected"},
                )
            duplicate = root / "duplicate-status.json"
            duplicate.write_text(
                '{"schema":1,"scenario":"session-chat-negative-capability",'
                '"status":"failed","status":"passed","reason":"validated",'
                '"validated_checks":10,"refs":"unchanged","receives":"unchanged",'
                '"golden_test":"bearer_push_starts_run_through_production_bootstrap",'
                '"golden_test_passes":1,"runner_exit_status":0}',
                encoding="utf-8",
            )
            summary = self._collect(root / "duplicate", duplicate)
            self.assertEqual(
                summary["sessionChatNegativeSummary"],
                {"status": "unknown", "reason": "rejected"},
            )


if __name__ == "__main__":
    unittest.main()
