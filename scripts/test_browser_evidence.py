"""Regression cases for retained browser credential checks."""

import base64
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import unittest
import tempfile
import zipfile


SPEC = importlib.util.spec_from_file_location(
    "browser_evidence", Path(__file__).with_name("check-browser-evidence.py")
)
EVIDENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EVIDENCE)


def archive(content):
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as target:
        target.writestr("trace.network", content)
    return output.getvalue()


class BrowserEvidenceTests(unittest.TestCase):
    def test_includes_rotated_cooking_credential(self):
        self.assertIn(b"cooking-model-rotated-fixture-sentinel-936f", EVIDENCE.VALUES)
        self.assertIn(b"cooking-inbound-rotated-fixture-sentinel-157a", EVIDENCE.VALUES)
        self.assertIn(b"cooking-relay-rotated-fixture-sentinel-482b", EVIDENCE.VALUES)

    def test_requires_retained_evidence(self):
        with self.assertRaises(ValueError):
            EVIDENCE.main([])
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(ValueError):
                EVIDENCE.main([root])

    def test_rejects_credentials_inside_retained_report_formats(self):
        for value in EVIDENCE.VALUES:
            for encoded in (value, base64.b64encode(value)):
                trace = archive(b'{"value":"' + encoded + b'"}')
                report = b"data:application/zip;base64," + base64.b64encode(trace)
                for content in (encoded, trace, archive(trace), report):
                    with self.subTest(size=len(content)):
                        with self.assertRaises(ValueError):
                            EVIDENCE.check_bytes(content, "fixture")

    def test_preserves_safe_trace_content(self):
        EVIDENCE.check_bytes(archive(b'{"value":"[REDACTED]"}'), "fixture")

    def test_rejects_uninspectable_archives(self):
        with self.assertRaises(zipfile.BadZipFile):
            EVIDENCE.check_bytes(b"PK\x03\x04broken", "fixture")
        with self.assertRaises(ValueError):
            EVIDENCE.check_bytes(archive(archive(archive(archive(b"safe")))), "fixture")

    def test_stream_redacts_raw_value_split_across_chunks(self):
        value = EVIDENCE.VALUES[0]
        source = io.BytesIO(b"prefix-" + value + b"-suffix")
        output = io.BytesIO()
        matched = EVIDENCE.redact_stream(source, output, chunk_bytes=3)
        self.assertTrue(matched)
        self.assertEqual(output.getvalue(), b"prefix-[REDACTED]-suffix")
        self.assertNotIn(value, output.getvalue())

    def test_stream_redacts_raw_base64_and_hex_variants(self):
        chunks = []
        for value in EVIDENCE.VALUES:
            chunks.extend(
                (
                    value,
                    base64.b64encode(value),
                    value.hex().encode("ascii"),
                    value.hex().upper().encode("ascii"),
                )
            )
        source_bytes = b"|".join(chunks)
        source = io.BytesIO(source_bytes)
        output = io.BytesIO()
        matched = EVIDENCE.redact_stream(source, output, chunk_bytes=1)
        self.assertTrue(matched)
        output_bytes = output.getvalue()
        for chunk in chunks:
            self.assertNotIn(chunk, output_bytes)
        self.assertEqual(output_bytes.count(b"[REDACTED]"), len(chunks))

    def test_stream_preserves_safe_passthrough(self):
        source_bytes = b"safe output without fixture credentials\n"
        output = io.BytesIO()
        self.assertFalse(EVIDENCE.redact_stream(io.BytesIO(source_bytes), output, chunk_bytes=2))
        self.assertEqual(output.getvalue(), source_bytes)

    def test_stream_flushes_progress_before_input_eof(self):
        class ChunkInput:
            def __init__(self):
                self.chunks = [b"first line\n", b"second line\n"]

            def read1(self, _size):
                return self.chunks.pop(0) if self.chunks else b""

        class FlushRecordingOutput(io.BytesIO):
            flush_count = 0

            def flush(self):
                self.flush_count += 1
                super().flush()

        output = FlushRecordingOutput()
        self.assertFalse(EVIDENCE.redact_stream(ChunkInput(), output, chunk_bytes=2))
        self.assertGreaterEqual(output.flush_count, 2)
        self.assertEqual(output.getvalue(), b"first line\nsecond line\n")

    def test_stream_cli_fails_after_redacting_input(self):
        value = EVIDENCE.VALUES[-1]
        prefix = b"x" * (EVIDENCE.STREAM_CHUNK_BYTES - 2)
        completed = subprocess.run(
            [sys.executable, str(Path(__file__).with_name("check-browser-evidence.py")), "--stream"],
            input=prefix + value + b"-after",
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertEqual(completed.returncode, 1)
        self.assertNotIn(value, completed.stdout)
        self.assertNotIn(value, completed.stderr)
        self.assertEqual(completed.stdout, prefix + b"[REDACTED]-after")


if __name__ == "__main__":
    unittest.main()
