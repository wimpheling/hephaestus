"""Focused tests for retained run snapshot sanitization."""

import importlib.util
from pathlib import Path
import unittest


SPEC = importlib.util.spec_from_file_location(
    "run_snapshot", Path(__file__).with_name("redact-run-snapshot.py")
)
SNAPSHOT = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(SNAPSHOT)


class RunSnapshotTests(unittest.TestCase):
    def test_decodes_and_redacts_vm_log_without_retaining_bytes(self):
        value = SNAPSHOT.EVIDENCE.VALUES[0]
        row = {
            "surface": "run_vm_log",
            "run_id": "00000000-0000-0000-0000-000000000001",
            "sequence": 1,
            "stream": "Stderr",
            "bytes": list(b"before-" + value + b"-after"),
        }
        safe = SNAPSHOT.sanitize_row(row)
        self.assertNotIn("bytes", safe)
        self.assertTrue(safe["credential_redacted"])
        self.assertNotIn(value.decode(), safe["text"])
        self.assertEqual(safe["text"], "before-[REDACTED]-after")

    def test_redacts_value_split_across_events_for_one_run_and_stream(self):
        value = SNAPSHOT.EVIDENCE.VALUES[0]
        split = len(value) // 2
        stream = SNAPSHOT.SnapshotStream()
        first = {
            "surface": "run_vm_log",
            "run_id": "run-a",
            "sequence": 1,
            "stream": "Stderr",
            "bytes": list(b"prefix-" + value[:split]),
        }
        second = {
            "surface": "run_vm_log",
            "run_id": "run-a",
            "sequence": 2,
            "stream": "Stderr",
            "bytes": list(value[split:] + b"-suffix"),
        }
        self.assertEqual(stream.accept(first), [])
        self.assertEqual(stream.accept(second), [])
        [safe] = stream.finish()
        self.assertNotIn(value.decode(), safe["text"])
        self.assertEqual(safe["text"], "prefix-[REDACTED]-suffix")
        self.assertEqual(safe["sequences"], [1, 2])

    def test_interleaved_runs_and_streams_are_never_joined(self):
        value = SNAPSHOT.EVIDENCE.VALUES[0]
        stream = SNAPSHOT.SnapshotStream()
        rows = [
            {
                "surface": "run_vm_log",
                "run_id": "run-a",
                "sequence": 1,
                "stream": "Stdout",
                "bytes": list(value[: len(value) // 2]),
            },
            {
                "surface": "run_vm_log",
                "run_id": "run-b",
                "sequence": 1,
                "stream": "Stdout",
                "bytes": list(value[len(value) // 2 :]),
            },
            {
                "surface": "run_vm_log",
                "run_id": "run-a",
                "sequence": 2,
                "stream": "Stderr",
                "bytes": list(value[len(value) // 2 :]),
            },
        ]
        emitted = []
        for row in rows:
            emitted.extend(stream.accept(row))
        emitted.extend(stream.finish())
        self.assertEqual(len(emitted), 3)
        self.assertTrue(all(value.decode() not in row["text"] for row in emitted))

    def test_rejects_boolean_bytes(self):
        with self.assertRaises(ValueError):
            SNAPSHOT.sanitize_row({"surface": "run_vm_log", "bytes": [True]})

    def test_accepts_non_log_metadata_without_payload_fields(self):
        row = {"surface": "run", "id": "run-id", "exit_signal": 9}
        self.assertEqual(SNAPSHOT.sanitize_row(row), row)

    def test_rejects_invalid_or_oversized_byte_arrays(self):
        with self.assertRaises(ValueError):
            SNAPSHOT.sanitize_row({"surface": "run_vm_log", "bytes": [256]})
        with self.assertRaises(ValueError):
            SNAPSHOT.sanitize_row(
                {"surface": "run_vm_log", "bytes": [0] * (SNAPSHOT.MAX_LOG_BYTES + 1)}
            )


if __name__ == "__main__":
    unittest.main()
