"""Regression coverage for the libkrun Unix-socket path budget."""

from __future__ import annotations

import os
from pathlib import Path
import shutil
import socket
import unittest
import uuid


ROOT = Path(__file__).parent
HARNESS = ROOT / "run-libkrun-integration.sh"
SUN_PATH_LIMIT = 108


class LibkrunRuntimeRootTests(unittest.TestCase):
    def test_runtime_root_is_a_short_disposable_sibling(self) -> None:
        source = HARNESS.read_text(encoding="utf-8")
        self.assertIn(
            'runtime_root="$(mktemp -d "${HEPHAESTUS_LIBKRUN_TMP_ROOT:-/tmp}/r.XXXXXX")"',
            source,
        )
        self.assertNotIn('${fixture_root}/runtime', source)
        self.assertEqual(source.count('HEPHAESTUS_LIBKRUN_RUNTIME_ROOT="${runtime_root}"'), 4)

        parent = Path("/tmp/hephaestus-libkrun")
        parent.mkdir(mode=0o700, exist_ok=True)
        suffix = uuid.uuid4().hex[:6]
        fixture_root = parent / f"h.{suffix}"
        runtime_root = parent / f"r.{suffix}"
        fixture_root.mkdir(mode=0o700)
        runtime_root.mkdir(mode=0o700)
        try:
            worker_id = "gateway-service-00000000-0000-0000-0000-000000000000"
            socket_name = "private-service.sock"
            short_path = runtime_root / worker_id / socket_name
            long_path = fixture_root / "runtime" / worker_id / socket_name
            self.assertLess(len(os.fsencode(str(short_path))) + 1, SUN_PATH_LIMIT)
            self.assertGreaterEqual(len(os.fsencode(str(long_path))) + 1, SUN_PATH_LIMIT)

            short_path.parent.mkdir(mode=0o700)
            listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                listener.bind(str(short_path))
            finally:
                listener.close()
            short_path.unlink()

            long_path.parent.mkdir(mode=0o700, parents=True)
            rejected = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                with self.assertRaises(OSError) as raised:
                    rejected.bind(str(long_path))
                self.assertTrue(
                    raised.exception.errno in (36, 22)
                    or "too long" in str(raised.exception).lower(),
                    raised.exception,
                )
            finally:
                rejected.close()
        finally:
            shutil.rmtree(fixture_root, ignore_errors=True)
            shutil.rmtree(runtime_root, ignore_errors=True)
        self.assertFalse(fixture_root.exists())
        self.assertFalse(runtime_root.exists())


if __name__ == "__main__":
    unittest.main()
