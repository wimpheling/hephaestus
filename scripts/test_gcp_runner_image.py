"""Focused contract tests for the immutable runner-image manifest."""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("gcp-runner-image-verify.py")
MANIFEST_WRITER = Path(__file__).with_name("gcp-runner-image-manifest.py")
PINS = {
    "--rust-version": "1.88.0",
    "--libkrun-tag": "v1.19.0",
    "--libkrun-revision": "9932c4b59d8f891e60c6aba20d22ebb99ceaa8e2",
    "--libkrunfw-tag": "v5.5.0",
    "--passt-revision": "386b5f5472b89769c025f5d5056348532a823b93",
    "--node-version": "v24.16.0",
    "--playwright-version": "1.62.0",
    "--oras-version": "1.3.3",
    "--oras-sha256": "9ce999f8d2de03fc03968b29d743077a58783e545e5eaa53917ca177352d0e59",
}


class RunnerImageManifestTests(unittest.TestCase):
    def make_manifest(self, root: Path) -> Path:
        required = [
            "/usr/bin/passt",
            "/usr/local/lib64/libkrun.so.1",
            "/usr/local/lib64/libkrunfw.so.5",
            "/usr/local/bin/node",
            "/srv/hephaestus/playwright-browsers",
            "/usr/local/libexec/hephaestus/gcp-runner-image-provision.sh",
            "/usr/local/libexec/hephaestus/gcp-runner-image-verify.py",
            "/usr/local/libexec/hephaestus/gcp-kvm-startup.sh",
            "/usr/local/libexec/hephaestus/gcp-runner-image-bake.sh",
            "/usr/local/libexec/hephaestus/gcp-runner-image-manifest.py",
        ]
        for item in required:
            target = root / item.lstrip("/")
            target.parent.mkdir(parents=True, exist_ok=True)
            if item.endswith("playwright-browsers"):
                target.mkdir()
            else:
                target.touch()
        marker = root / "etc/hephaestus/runner-image-required"
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.touch()
        document = {
            "schema": 1,
            "kind": "hephaestus-gcp-runner",
            "pins": {
                "rust_version": "1.88.0",
                "libkrun_tag": "v1.19.0",
                "libkrun_revision": PINS["--libkrun-revision"],
                "libkrunfw_tag": "v5.5.0",
                "passt_revision": PINS["--passt-revision"],
                "node_version": "v24.16.0",
                "playwright": "1.62.0",
                "oras_version": "1.3.3",
                "oras_sha256": PINS["--oras-sha256"],
            },
            "browser_lock_sha256": "a" * 64,
            "browser_version": "Chromium 1.2.3",
            "recipe_sha256": "",
            "verifier_sha256": "",
            "startup_sha256": "",
            "bake_sha256": "",
            "manifest_generator_sha256": "",
            "required_paths": required,
        }
        for field, item in {
            "recipe_sha256": "/usr/local/libexec/hephaestus/gcp-runner-image-provision.sh",
            "verifier_sha256": "/usr/local/libexec/hephaestus/gcp-runner-image-verify.py",
            "startup_sha256": "/usr/local/libexec/hephaestus/gcp-kvm-startup.sh",
            "bake_sha256": "/usr/local/libexec/hephaestus/gcp-runner-image-bake.sh",
            "manifest_generator_sha256": "/usr/local/libexec/hephaestus/gcp-runner-image-manifest.py",
        }.items():
            target = root / item.lstrip("/")
            target.write_text(field, encoding="utf-8")
            document[field] = hashlib.sha256(target.read_bytes()).hexdigest()
        encoded = json.dumps(document, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
        document["manifest_sha256"] = hashlib.sha256(encoded).hexdigest()
        path = root / "manifest.json"
        path.write_text(json.dumps(document), encoding="utf-8")
        return path

    def execute(self, root: Path, manifest: Path, *extra: str) -> subprocess.CompletedProcess[str]:
        args = [sys.executable, str(SCRIPT), str(manifest), "--root", str(root)]
        for key, value in PINS.items():
            args.extend((key, value))
        args.extend(extra)
        return subprocess.run(args, check=False, text=True, capture_output=True)

    def test_valid_manifest_accepts_prebuilt_paths(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.execute(root, self.make_manifest(root))
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_fingerprint_mismatch_fail_closes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = self.make_manifest(root)
            document = json.loads(manifest.read_text(encoding="utf-8"))
            document["pins"]["node_version"] = "v99.0.0"
            manifest.write_text(json.dumps(document), encoding="utf-8")
            result = self.execute(root, manifest)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("fingerprint mismatch", result.stderr)

    def test_dependency_version_mismatch_fail_closes_after_resealing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = self.make_manifest(root)
            document = json.loads(manifest.read_text(encoding="utf-8"))
            document["pins"]["node_version"] = "v99.0.0"
            unsigned = dict(document)
            unsigned.pop("manifest_sha256")
            document["manifest_sha256"] = hashlib.sha256(
                json.dumps(unsigned, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
            ).hexdigest()
            manifest.write_text(json.dumps(document), encoding="utf-8")
            result = self.execute(root, manifest)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("dependency pins", result.stderr)

    def test_missing_baked_browser_fail_closes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = self.make_manifest(root)
            (root / "srv/hephaestus/playwright-browsers").rmdir()
            result = self.execute(root, manifest)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("required path is missing", result.stderr)

    def test_only_node_and_soname_symlinks_are_allowed_inside_install_roots(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = self.make_manifest(root)
            node_link = root / "usr/local/bin/node"
            node_link.unlink()
            node_target = root / "opt/hephaestus/node-v24.16.0/bin/node"
            node_target.parent.mkdir(parents=True)
            node_target.touch()
            node_link.symlink_to("../../../opt/hephaestus/node-v24.16.0/bin/node")
            lib_link = root / "usr/local/lib64/libkrun.so.1"
            lib_link.unlink()
            lib_target = root / "usr/local/lib64/libkrun.so.1.0"
            lib_target.touch()
            lib_link.symlink_to("libkrun.so.1.0")
            result = self.execute(root, manifest)
            self.assertEqual(result.returncode, 0, result.stderr)

            node_link.unlink()
            node_link.symlink_to("/etc/passwd")
            result = self.execute(root, manifest)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("escapes its install root", result.stderr)

    def test_startup_has_explicit_prebuilt_skip_and_bake_boundary(self) -> None:
        startup = SCRIPT.with_name("gcp-kvm-startup.sh").read_text(encoding="utf-8")
        self.assertIn('if [[ "$runner_image_ready" == true ]]; then', startup)
        self.assertIn('phase=libkrun status=prebuilt', startup)
        self.assertIn('if [[ "${HEPH_GCP_IMAGE_BAKE:-0}" == 1 ]]; then', startup)
        self.assertIn("runner-image-selection metadata must explicitly be custom or stock", startup)
        self.assertLess(startup.index("phase_start image-bake-ready"), startup.index("phase_start checkout"))
        subprocess.run(["bash", "-n", str(SCRIPT.with_name("gcp-kvm-startup.sh"))], check=True)

    def test_cooking_consumes_verified_browser_and_host_tools(self) -> None:
        cooking = SCRIPT.with_name("gcp-cooking-run.sh").read_text(encoding="utf-8")
        self.assertIn('runner_image_verified="${HEPH_GCP_RUNNER_IMAGE_VERIFIED:-false}"', cooking)
        self.assertIn("checked-out browser lock does not match the baked browser assets", cooking)
        self.assertIn("verified runner image Chromium executable is missing", cooking)
        self.assertIn("npx playwright install-deps chromium", cooking)
        self.assertIn("if [[ \"$runner_image_verified\" == true ]]; then", cooking)

    def test_manifest_writer_generates_verifiable_fingerprint(self) -> None:
        environment = {
            "HEPH_IMAGE_RUST_VERSION": "1.88.0",
            "HEPH_IMAGE_LIBKRUN_TAG": "v1.19.0",
            "HEPH_IMAGE_LIBKRUN_REVISION": PINS["--libkrun-revision"],
            "HEPH_IMAGE_LIBKRUNFW_TAG": "v5.5.0",
            "HEPH_IMAGE_PASST_REVISION": PINS["--passt-revision"],
            "HEPH_IMAGE_NODE_VERSION": "v24.16.0",
            "HEPH_IMAGE_NODE_SHA256": "b" * 64,
            "HEPH_IMAGE_PLAYWRIGHT_VERSION": "1.62.0",
            "HEPH_IMAGE_ORAS_VERSION": "1.3.3",
            "HEPH_IMAGE_ORAS_SHA256": PINS["--oras-sha256"],
        }
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "manifest.json"
            args = [
                sys.executable, str(MANIFEST_WRITER), "--output", str(output),
                "--repository-sha", "c" * 40, "--browser-lock-sha256", "d" * 64,
                "--browser-version", "Chromium 1.2.3",
                "--recipe-sha256", "e" * 64, "--startup-sha256", "f" * 64,
                "--bake-sha256", "0" * 64, "--verifier-sha256", "1" * 64,
                "--manifest-generator-sha256", "2" * 64,
            ]
            result = subprocess.run(args, env={**os.environ, **environment}, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            document = json.loads(output.read_text(encoding="utf-8"))
            unsigned = dict(document)
            fingerprint = unsigned.pop("manifest_sha256")
            canonical = json.dumps(unsigned, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
            self.assertEqual(fingerprint, hashlib.sha256(canonical).hexdigest())


if __name__ == "__main__":
    unittest.main()
