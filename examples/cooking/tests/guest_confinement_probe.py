#!/usr/bin/env python3
"""Host-side descriptor generator and tests for the cooking guest probe.

The runtime scanner lives separately in ``guest_confinement`` so
it can be copied into a diagnostic guest without host fixture code or values.
"""

from __future__ import annotations

import argparse
import base64
import contextlib
import errno
import hashlib
import io
import json
from pathlib import Path
import socket
import stat
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import guest_confinement as runtime

EXPECTED_LABELS = runtime.EXPECTED_LABELS
EXPECTED_ENCODINGS = runtime.EXPECTED_ENCODINGS


def encoding_values(value: bytes) -> tuple[tuple[str, bytes], ...]:
    return (
        ("raw", value),
        ("base64", base64.b64encode(value)),
        ("hex_lower", value.hex().encode()),
        ("hex_upper", value.hex().upper().encode()),
    )


def descriptors(candidates: dict[str, bytes]) -> list[dict[str, object]]:
    """Generate fingerprints for exactly the six approved fixture labels."""
    if set(candidates) != set(EXPECTED_LABELS):
        raise runtime.ProbeError("descriptor input must contain exactly six fixture labels")
    records: list[dict[str, object]] = []
    for label in EXPECTED_LABELS:
        value = candidates[label]
        if not 4 <= len(value) <= runtime.CHUNK_SIZE:
            raise runtime.ProbeError("fixture value is outside the bounded descriptor size")
        for encoding, encoded in encoding_values(value):
            records.append(
                {
                    "label": label,
                    "encoding": encoding,
                    "length": len(encoded),
                    "sha256": "sha256:" + hashlib.sha256(encoded).hexdigest(),
                    "prefix4": encoded[:4].hex(),
                }
            )
    return records


def parse_candidates(values: list[str]) -> dict[str, bytes]:
    candidates: dict[str, bytes] = {}
    for argument in values:
        label, separator, filename = argument.partition("=")
        if not separator or label not in EXPECTED_LABELS or label in candidates:
            raise runtime.ProbeError("candidate arguments must be unique approved labels")
        path = Path(filename)
        try:
            if not path.is_file() or path.is_symlink():
                raise runtime.ProbeError("candidate path is not a regular file")
            with path.open("rb") as stream:
                content = stream.read(runtime.CHUNK_SIZE + 1)
        except runtime.ProbeError:
            raise
        except OSError as error:
            raise runtime.ProbeError("candidate path could not be read") from error
        if not 4 <= len(content) <= runtime.CHUNK_SIZE:
            raise runtime.ProbeError("candidate value is outside the bounded size")
        candidates[label] = content
    if set(candidates) != set(EXPECTED_LABELS):
        raise runtime.ProbeError("exactly six candidate files are required")
    return candidates


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.values = {
            label: (f"probe-{label}-" + "0123456789" * 12).encode()
            for label in EXPECTED_LABELS
        }
        self.records = descriptors(self.values)

    def test_all_encodings_and_chunk_boundaries(self):
        for label, value in self.values.items():
            for encoding, encoded in encoding_values(value):
                self.assertIn((label, encoding), runtime._scan_bytes(encoded, self.records))
                split = b"x" * (runtime.CHUNK_SIZE - 2) + encoded + b"y"
                self.assertIn(
                    (label, encoding),
                    runtime.scan_stream(io.BytesIO(split), self.records),
                )

    def test_process_environment_and_actual_subprocess_argv(self):
        value = self.values["model"]
        import os

        os.environ["PROBE_FIXTURE_VALUE"] = value.decode()
        try:
            result = runtime.scan_process(self.records, ["agent", value.decode()])
        finally:
            del os.environ["PROBE_FIXTURE_VALUE"]
        self.assertIn("model:raw", result["environment"])
        self.assertIn("model:raw", result["argv"])
        self.assertNotIn(value.decode(), json.dumps(result))
        with tempfile.TemporaryDirectory() as temporary:
            descriptor_file = Path(temporary) / "descriptors.json"
            descriptor_file.write_text(
                json.dumps({"version": 1, "descriptors": self.records})
            )
            completed = subprocess.run(
                [
                    sys.executable,
                    __file__,
                    "--scan-descriptors",
                    str(descriptor_file),
                    "--surface",
                    "/proc/self/cmdline",
                    "--probe-arg",
                    value.decode(),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(completed.returncode, 0)
            self.assertNotIn(value.decode(), completed.stdout + completed.stderr)

    def test_nested_files_filename_poison_runtime_credential_and_symlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            approved = root / "work"
            approved.mkdir()
            nested = approved / "nested" / "output.bin"
            nested.parent.mkdir()
            nested.write_bytes(encoding_values(self.values["relay"])[1][1])
            filename_canary = approved / (self.values["inbound"].decode() + ".txt")
            filename_canary.write_bytes(b"ordinary")
            runtime_credential = approved / ".runtime-credential"
            runtime_credential.write_bytes(b"r" * 32)
            allowed = (str(approved),)
            result = runtime.scan_surface(approved, self.records, allowed_roots=allowed)
            self.assertIn("relay:base64", result["matches"])
            self.assertIn("inbound:raw", result["matches"])
            self.assertNotIn(self.values["inbound"].decode(), json.dumps(result))
            runtime_result = runtime.scan_surface(
                runtime_credential, self.records, allowed_roots=allowed
            )
            self.assertEqual(runtime_result["inventory"][0]["size"], 32)
            escape = approved / "escape"
            escape.symlink_to(root / "outside")
            with self.assertRaises(runtime.ProbeError):
                runtime.scan_surface(approved, self.records, allowed_roots=allowed)

    def test_empty_missing_and_aggregate_bounds_fail(self):
        with self.assertRaises(runtime.ProbeError):
            runtime.runtime_probe(self.records, ())
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            allowed = (str(root),)
            with self.assertRaises(runtime.ProbeError):
                runtime.runtime_probe(self.records, (str(root),))
            first = root / "first"
            second = root / "second"
            first.write_bytes(b"a" * 20)
            second.write_bytes(b"b" * 20)
            old_limit = runtime.MAX_TOTAL_BYTES
            runtime.MAX_TOTAL_BYTES = 64
            try:
                budget = runtime.ScanBudget()
                runtime.scan_surface(first, self.records, budget, allowed_roots=allowed)
                with self.assertRaises(runtime.ProbeError):
                    runtime.scan_surface(second, self.records, budget, allowed_roots=allowed)
            finally:
                runtime.MAX_TOTAL_BYTES = old_limit

    def test_sockets_require_an_explicit_expected_path(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            socket_path = root / "broker.sock"
            server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                server.bind(str(socket_path))
                allowed = (str(root),)
                with self.assertRaises(runtime.ProbeError):
                    runtime.scan_surface(root, self.records, allowed_roots=allowed)
                result = runtime.scan_surface(
                    socket_path,
                    self.records,
                    allowed_roots=allowed,
                    expected_sockets=(str(socket_path),),
                )
                self.assertEqual(result["inventory"][0]["kind"], "socket")
            finally:
                server.close()

    def test_in_process_before_and_after_evidence_is_nonempty(self):
        with tempfile.TemporaryDirectory() as temporary:
            surface = Path(temporary) / "control.json"
            surface.write_bytes(b"{}")
            result, before, after = runtime.run_with_probe(
                self.records,
                (str(surface),),
                lambda: "completed",
            )
            self.assertEqual(result, "completed")
            self.assertGreater(before["scanned_files"], 0)
            self.assertGreater(after["scanned_files"], 0)

    def test_os_errors_are_not_chained_into_diagnostics(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = runtime.os.lstat

            def denied(path):
                if path == root:
                    raise OSError("path-canary-must-not-escape")
                return original(path)

            with patch.object(runtime.os, "lstat", side_effect=denied):
                with self.assertRaises(runtime.ProbeError) as raised:
                    runtime.scan_surface(root, self.records, allowed_roots=(str(root),))
            self.assertNotIn("path-canary", str(raised.exception))
            self.assertIsNone(raised.exception.__cause__)

    def test_enumeration_diagnostic_has_only_surface_and_errno_codes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)

            def denied_walk(_path, *, onerror, **_kwargs):
                onerror(PermissionError(errno.EACCES, "path-canary-must-not-escape"))
                return iter(())

            with patch.object(runtime.os, "walk", side_effect=denied_walk):
                with self.assertRaises(runtime.ProbeError) as raised:
                    runtime.scan_surface(
                        root,
                        self.records,
                        allowed_roots=(str(root),),
                        surface_code="proc-env",
                    )
            self.assertEqual(raised.exception.surface_code, "proc-env")
            self.assertEqual(raised.exception.errno_code, "permission")
            self.assertNotIn("path-canary", str(raised.exception))
            self.assertIsNone(raised.exception.__cause__)

    def test_ext4_protected_metadata_is_explicit_and_state_files_are_scanned(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            state = root / "state.db"
            state.write_bytes(b"safe state")
            lost_found = root / "lost+found"
            lost_found.mkdir(mode=0o700)
            lost_found.chmod(0o700)
            protected_mode = 0o700

            original_lstat = runtime.os.lstat

            def state_lstat(path):
                if Path(path) == lost_found:
                    return SimpleNamespace(
                        st_mode=stat.S_IFDIR | protected_mode,
                        st_uid=0,
                    )
                return original_lstat(path)

            def denied_walk(path, *, onerror, **_kwargs):
                onerror(PermissionError(errno.EACCES, "Permission denied", str(lost_found)))
                yield (str(path), [], [state.name])

            with (
                patch.object(runtime.os, "walk", side_effect=denied_walk),
                patch.object(runtime.os, "lstat", side_effect=state_lstat),
            ):
                result = runtime.scan_surface(
                    root,
                    self.records,
                    allowed_roots=(str(root),),
                    surface_code="state",
                    expected_protected_metadata=(str(lost_found),),
                )
            self.assertEqual(result["files"], 1)
            self.assertEqual(result["protected_metadata"][0]["kind"], "directory")
            self.assertEqual(result["protected_metadata"][0]["mode"], 0o700)
            self.assertEqual(result["protected_metadata"][0]["uid"], 0)
            self.assertEqual(result["protected_metadata"][0]["access"], "denied")

            protected_mode = 0o755
            with (
                patch.object(runtime.os, "walk", side_effect=denied_walk),
                patch.object(runtime.os, "lstat", side_effect=state_lstat),
            ):
                with self.assertRaises(runtime.ProbeError) as raised:
                    runtime.scan_surface(
                        root,
                        self.records,
                        allowed_roots=(str(root),),
                        surface_code="state",
                        expected_protected_metadata=(str(lost_found),),
                    )
            self.assertEqual(raised.exception.errno_code, "permission")

    def test_cli_requires_surfaces_and_clean_runtime_credential_succeeds(self):
        with tempfile.TemporaryDirectory() as temporary:
            descriptor_file = Path(temporary) / "descriptors.json"
            descriptor_file.write_text(
                json.dumps({"version": 1, "descriptors": self.records})
            )
            base = [
                sys.executable,
                __file__,
                "--scan-descriptors",
                str(descriptor_file),
            ]
            no_surface = subprocess.run(base, capture_output=True, text=True, check=False)
            self.assertNotEqual(no_surface.returncode, 0)
            missing = subprocess.run(
                base + ["--surface", "/proc/self/missing"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(missing.returncode, 0)
            clean = subprocess.run(
                base + ["--surface", "/proc/self/cmdline"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(clean.returncode, 0)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--candidate", action="append", default=[])
    parser.add_argument("--output", type=Path)
    parser.add_argument("--scan-descriptors", type=Path)
    parser.add_argument("--surface", action="append", default=[])
    parser.add_argument("--probe-arg", action="append", default=[])
    args = parser.parse_args()
    if args.self_test:
        return 0 if unittest.main(argv=[__file__], exit=False).result.wasSuccessful() else 1
    if args.candidate:
        output = {"version": 1, "descriptors": descriptors(parse_candidates(args.candidate))}
        encoded = json.dumps(output, sort_keys=True, separators=(",", ":"))
        try:
            if args.output:
                args.output.write_text(encoded + "\n")
            else:
                print(encoded)
        except OSError as error:
            raise runtime.ProbeError("descriptor output could not be written") from error
        return 0
    if not args.scan_descriptors:
        parser.error("use --self-test, --candidate, or --scan-descriptors")
    if not args.surface:
        parser.error("at least one required --surface is needed")
    try:
        records = json.loads(args.scan_descriptors.read_text())["descriptors"]
    except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        raise runtime.ProbeError("descriptor file could not be loaded") from error
    result = runtime.runtime_probe(records, tuple(args.surface), list(sys.argv) + args.probe_arg)
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except runtime.ProbeError as error:
        print(f"guest probe failed: {error}", file=sys.stderr)
        raise SystemExit(2)
