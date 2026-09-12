"""Focused tests for the optional encrypted diagnostics-triage export."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import os
import stat
import subprocess
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).parent
HELPER = ROOT / "export-encrypted-diagnostics.sh"
WORKFLOW = ROOT.parent / ".github" / "workflows" / "cooking-e2e.yml"
COLLECTOR_SPEC = importlib.util.spec_from_file_location(
    "gcp_diagnostics_collector", ROOT / "collect-cooking-diagnostics.py"
)
assert COLLECTOR_SPEC is not None and COLLECTOR_SPEC.loader is not None
COLLECTOR = importlib.util.module_from_spec(COLLECTOR_SPEC)
COLLECTOR_SPEC.loader.exec_module(COLLECTOR)


class EncryptedDiagnosticsTests(unittest.TestCase):
    def _run(self, *args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        if env is not None:
            environment.update(env)
        return subprocess.run(
            ["bash", str(HELPER), *args],
            text=True,
            capture_output=True,
            env=environment,
            check=False,
        )

    def _archive(self, root: Path, *, unsafe: bool = False) -> Path:
        source = root / "serial.log"
        source.write_text("safe retained serial evidence\n", encoding="utf-8")
        digest = hashlib.sha256(source.read_bytes()).hexdigest()
        manifest = {
            "schema": 1,
            "credentialScan": "passed",
            "sources": [{
                "label": "serial", "path": "sources/serial.log",
                "bytes": source.stat().st_size, "sha256": digest,
            }],
        }
        manifest_path = root / "manifest.json"
        manifest_path.write_text(json.dumps(manifest) + "\n", encoding="utf-8")
        archive = root.parent / "diagnostics.tar.gz"
        with tarfile.open(archive, "w:gz") as output:
            output.add(manifest_path, arcname="cooking-diagnostics/manifest.json", recursive=False)
            output.add(source, arcname="cooking-diagnostics/sources/serial.log", recursive=False)
            if unsafe:
                link = tarfile.TarInfo("cooking-diagnostics/sources/unsafe")
                link.type = tarfile.SYMTYPE
                link.linkname = "/etc/passwd"
                output.addfile(link)
        return archive

    def _recipient(self, root: Path) -> tuple[Path, Path]:
        directory = root / "recipient"
        result = self._run("generate-recipient", str(directory))
        self.assertEqual(result.returncode, 0, result.stderr)
        key = directory / "recipient-key.pem"
        cert = directory / "recipient-cert.pem"
        self.assertEqual(stat.S_IMODE(directory.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(key.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(cert.stat().st_mode), 0o644)
        self.assertNotIn("PRIVATE KEY", cert.read_text(encoding="ascii"))
        return key, cert

    def _status(self, archive: Path, root: Path) -> Path:
        status = root / "status.json"
        status.write_text(
            json.dumps(
                {
                    "cleanup": "verified-absent",
                    "upload": "verified-by-download",
                    "download": "passed",
                    "scan": "passed",
                    "archiveBytes": archive.stat().st_size,
                    "archiveSha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
                }
            ),
            encoding="utf-8",
        )
        return status

    def test_roundtrip_and_local_validation(self):
        with tempfile.TemporaryDirectory(prefix="heph-encrypted-export-") as directory:
            root = Path(directory)
            input_root = root / "input"
            input_root.mkdir()
            archive = self._archive(input_root)
            key, cert = self._recipient(root)
            encrypted = root / "gcp-diagnostics.tar.gz.cms"
            result = self._run("export", str(archive), str(cert), str(encrypted), str(self._status(archive, root)))
            self.assertEqual(result.returncode, 0, result.stderr)
            original_ciphertext = encrypted.read_bytes()
            result = self._run("export", str(archive), str(cert), str(encrypted), str(self._status(archive, root)))
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(encrypted.read_bytes(), original_ciphertext)
            self.assertEqual(stat.S_IMODE(encrypted.stat().st_mode), 0o600)
            decrypted = root / "decrypted.tar.gz"
            result = subprocess.run(
                [
                    "openssl", "cms", "-decrypt", "-inform", "DER",
                    "-in", str(encrypted), "-recip", str(cert), "-inkey", str(key),
                    "-out", str(decrypted),
                ],
                capture_output=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(decrypted.read_bytes(), archive.read_bytes())
            result = self._run("validate", str(decrypted))
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_canonical_collector_bundle_exports(self):
        with tempfile.TemporaryDirectory(prefix="heph-encrypted-canonical-") as directory:
            root = Path(directory)
            source = root / "serial-input.log"
            source.write_text("canonical retained serial evidence\n", encoding="utf-8")
            output_dir = root / "canonical-bundle"
            archive = root / "canonical.tar.gz"
            self.assertEqual(
                COLLECTOR.collect(output_dir, [f"serial={source}"], None, None, archive),
                0,
            )
            _key, cert = self._recipient(root)
            encrypted = root / "canonical.cms"
            result = self._run(
                "export", str(archive), str(cert), str(encrypted), str(self._status(archive, root))
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_tamper_fails_decryption(self):
        with tempfile.TemporaryDirectory(prefix="heph-encrypted-tamper-") as directory:
            root = Path(directory)
            input_root = root / "input"
            input_root.mkdir()
            archive = self._archive(input_root)
            key, cert = self._recipient(root)
            encrypted = root / "encrypted.cms"
            self.assertEqual(
                self._run("export", str(archive), str(cert), str(encrypted), str(self._status(archive, root))).returncode,
                0,
            )
            data = bytearray(encrypted.read_bytes())
            data[-1] ^= 1
            encrypted.write_bytes(data)
            result = subprocess.run(
                [
                    "openssl", "cms", "-decrypt", "-inform", "DER",
                    "-in", str(encrypted), "-recip", str(cert), "-inkey", str(key),
                ],
                capture_output=True, check=False,
            )
            self.assertNotEqual(result.returncode, 0)

    def test_unsafe_archive_and_private_key_input_fail_closed(self):
        with tempfile.TemporaryDirectory(prefix="heph-encrypted-reject-") as directory:
            root = Path(directory)
            input_root = root / "input"
            input_root.mkdir()
            unsafe = self._archive(input_root, unsafe=True)
            key, cert = self._recipient(root)
            output = root / "unsafe.cms"
            temporary = root / "tmp"
            temporary.mkdir()
            result = self._run(
                "export", str(unsafe), str(cert), str(output), str(self._status(unsafe, root)),
                env={"TMPDIR": str(temporary)},
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())
            self.assertEqual(list(temporary.iterdir()), [])
            safe_root = root / "safe"
            safe_root.mkdir()
            safe = self._archive(safe_root)
            result = self._run("export", str(safe), str(key), str(output), str(self._status(safe, root)))
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())

    def test_unmanifested_member_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix="heph-encrypted-members-") as directory:
            root = Path(directory)
            input_root = root / "input"
            input_root.mkdir()
            archive = self._archive(input_root)
            rewritten = root / "unmanifested.tar.gz"
            with tarfile.open(archive, "r:gz") as source, tarfile.open(rewritten, "w:gz") as output:
                for member in source.getmembers():
                    output.addfile(member, source.extractfile(member) if member.isfile() else None)
                extra = tarfile.TarInfo("cooking-diagnostics/sources/extra")
                extra_data = b"not listed in the manifest\n"
                extra.size = len(extra_data)
                import io
                output.addfile(extra, io.BytesIO(extra_data))
            _key, cert = self._recipient(root)
            result = self._run(
                "export", str(rewritten), str(cert), str(root / "extra.cms"), str(self._status(rewritten, root))
            )
            self.assertNotEqual(result.returncode, 0)

    def test_aggregate_expansion_bound_is_enforced(self):
        with tempfile.TemporaryDirectory(prefix="heph-encrypted-tarbomb-") as directory:
            root = Path(directory)
            archive = root / "tarbomb.tar.gz"
            import io
            with tarfile.open(archive, "w:gz") as output:
                for index in range(9):
                    payload = b"x" * (15 * 1024 * 1024)
                    member = tarfile.TarInfo(f"cooking-diagnostics/sources/large-{index}")
                    member.size = len(payload)
                    output.addfile(member, io.BytesIO(payload))
            _key, cert = self._recipient(root)
            status = self._status(archive, root)
            result = self._run("export", str(archive), str(cert), str(root / "tarbomb.cms"), str(status))
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((root / "tarbomb.cms").exists())

    def test_duplicate_member_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix="heph-encrypted-duplicate-") as directory:
            root = Path(directory)
            input_root = root / "input"
            input_root.mkdir()
            archive = self._archive(input_root)
            duplicate = root / "duplicate.tar.gz"
            with tarfile.open(archive, "r:gz") as source, tarfile.open(duplicate, "w:gz") as output:
                members = source.getmembers()
                for member in members:
                    output.addfile(member, source.extractfile(member) if member.isfile() else None)
                member = members[-1]
                data = source.extractfile(member).read()
                import io
                output.addfile(member, io.BytesIO(data))
            _key, cert = self._recipient(root)
            result = self._run(
                "export", str(duplicate), str(cert), str(root / "duplicate.cms"), str(self._status(duplicate, root))
            )
            self.assertNotEqual(result.returncode, 0)

    def test_unverified_download_status_blocks_export(self):
        with tempfile.TemporaryDirectory(prefix="heph-encrypted-status-") as directory:
            root = Path(directory)
            input_root = root / "input"
            input_root.mkdir()
            archive = self._archive(input_root)
            _key, cert = self._recipient(root)
            status = self._status(archive, root)
            value = json.loads(status.read_text(encoding="utf-8"))
            value["cleanup"] = "unverified"
            status.write_text(json.dumps(value), encoding="utf-8")
            output = root / "blocked.cms"
            result = self._run("export", str(archive), str(cert), str(output), str(status))
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())

    def test_workflow_limits_export_to_triage_and_ciphertext(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("diagnostics_recipient_cert:", workflow)
        self.assertIn("inputs.cloud_mode == 'diagnostics-triage' && inputs.diagnostics_recipient_cert != ''", workflow)
        self.assertIn("steps.encrypt_diagnostics.outcome == 'success'", workflow)
        self.assertIn("gcp-diagnostics.tar.gz.cms", workflow)
        self.assertIn("retention-days: 1", workflow)
        self.assertNotIn("path: ${{ runner.temp }}/gcp-diagnostics.tar.gz\n", workflow)
        self.assertIn("PRIVATE KEY", workflow)


if __name__ == "__main__":
    unittest.main()
