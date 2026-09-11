"""Focused tests for typed HEPH_COOKING_RETRY evidence projection."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location(
    "cooking_diagnostics", Path(__file__).with_name("collect-cooking-diagnostics.py")
)
COLLECTOR = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(COLLECTOR)


EVENT_ID = "00000000-0000-4000-8000-000000000001"
ATTEMPT_ID = "00000000-0000-4000-8000-000000000002"
RUN_ID = "00000000-0000-4000-8000-000000000003"


def marker(**overrides: str) -> str:
    values = {
        "event": "terminal",
        "classification": "retry-terminal-failed",
        "lookup_status": "ok",
        "event_id": EVENT_ID,
        "attempt_id": ATTEMPT_ID,
        "attempt_number": "2",
        "run_id": RUN_ID,
        "attempt_state": "failed",
        "run_state": "failed",
        "run_outcome": "failed",
        "exit_code": "7",
        "exit_signal": "none",
    }
    values.update(overrides)
    return "HEPH_COOKING_RETRY " + " ".join(f"{key}={value}" for key, value in values.items())


class CookingRetryProjectionTests(unittest.TestCase):
    def project(self, value: str) -> str:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "serial.log"
            destination = root / "retained"
            source.write_text(value + "\n", encoding="utf-8")
            destination.parent.mkdir(parents=True, exist_ok=True)
            COLLECTOR._project_text(source, destination)
            return destination.read_text(encoding="utf-8")

    def test_projects_each_terminal_retry_classification_and_lookup(self):
        cases = (
            ({}, "retry-terminal-failed", "lookup_status=ok", "attempt_state=failed"),
            (
                {"classification": "retry-terminal-uncertain", "attempt_state": "uncertain"},
                "retry-terminal-uncertain",
                "lookup_status=ok",
                "attempt_state=uncertain",
            ),
            (
                {
                    "classification": "retry-completion-timeout",
                    "lookup_status": "query-timeout",
                    "attempt_id": "unknown",
                    "attempt_number": "unknown",
                    "attempt_state": "unknown",
                    "run_state": "unknown",
                    "run_outcome": "none",
                    "exit_code": "unknown",
                    "exit_signal": "none",
                },
                "retry-completion-timeout",
                "lookup_status=query-timeout",
                "attempt_number=unknown",
            ),
            (
                {
                    "classification": "retry-terminal-unresolved",
                    "lookup_status": "query-failed",
                    "attempt_id": "unknown",
                    "attempt_number": "unknown",
                    "attempt_state": "unknown",
                    "run_state": "unknown",
                    "run_outcome": "none",
                    "exit_code": "none",
                    "exit_signal": "unknown",
                },
                "retry-terminal-unresolved",
                "lookup_status=query-failed",
                "attempt_id=unknown",
            ),
        )
        for overrides, classification, lookup, state in cases:
            with self.subTest(classification=classification):
                retained = self.project(marker(**overrides))
                self.assertIn(f"classification={classification}", retained)
                self.assertIn(lookup, retained)
                self.assertIn(state, retained)
                self.assertNotIn("failure=", retained)
                self.assertNotIn("payload=", retained)

    def test_rejects_malformed_or_embedded_retry_markers(self):
        malformed = (
            marker(classification="retry-terminal-failed;payload=secret"),
            marker(exit_code="password=secret"),
            marker() + " failure=freeform",
            '"' + marker() + '"',
        )
        for value in malformed:
            with self.subTest(value=value):
                with self.assertRaises(COLLECTOR.CollectionError):
                    self.project(value)

    def test_accepts_exit_boundaries_and_rejects_unbounded_values(self):
        for field, maximum in (("exit_code", "255"), ("exit_signal", "64")):
            for value in ("0", maximum):
                with self.subTest(field=field, value=value):
                    self.project(marker(**{field: value}))
            over = str(int(maximum) + 1)
            for value in ("-1", over):
                with self.subTest(field=field, value=value):
                    with self.assertRaises(COLLECTOR.CollectionError):
                        self.project(marker(**{field: value}))

    def test_lineage_exit_fields_are_nullable_and_old_rows_remain_valid(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "lineage.jsonl"
            destination = root / "retained.jsonl"
            old_row = {
                "event_id": EVENT_ID,
                "attempt_id": ATTEMPT_ID,
                "attempt_run_id": RUN_ID,
                "attempt_number": 2,
                "attempt_state": "failed",
                "run_state": "failed",
                "run_outcome": "failed",
            }
            nullable_row = {**old_row, "exit_code": None, "exit_signal": None}
            source.write_text(
                json.dumps(old_row) + "\n" + json.dumps(nullable_row) + "\n", encoding="utf-8"
            )
            rows, _ = COLLECTOR._canonical_snapshot(source, destination)
            self.assertEqual(rows, 2)
            retained = [json.loads(line) for line in destination.read_text().splitlines()]
            self.assertNotIn("exit_code", retained[0])
            self.assertIsNone(retained[1]["exit_code"])
            self.assertIsNone(retained[1]["exit_signal"])

            for field in ("exit_code", "exit_signal"):
                over = COLLECTOR.EXIT_LIMITS[field] + 1
                for value in (-1, over):
                    invalid = {**old_row, field: value}
                    source.write_text(json.dumps(invalid) + "\n", encoding="utf-8")
                    with self.subTest(field=field, value=value):
                        with self.assertRaises(COLLECTOR.CollectionError):
                            COLLECTOR._canonical_snapshot(source, destination)


if __name__ == "__main__":
    unittest.main()
