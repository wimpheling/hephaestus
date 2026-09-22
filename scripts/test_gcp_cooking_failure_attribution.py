"""Focused tests for safe first-workload-failure attribution."""

from __future__ import annotations

import json
import hashlib
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
SCRIPT = ROOT / "gcp-cooking-run.sh"


class CookingFailureAttributionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        source = SCRIPT.read_text(encoding="utf-8")
        cls.source = source
        start = source.index("record_workload_first_failure() {")
        end = source.index("\nrequire_command()", start)
        cls.function = source[start:end]
        start = source.index("validate_runner_image_manifest() {")
        end = source.index("\ncase \"$cooking_scenario\"", start)
        cls.manifest_function = source[start:end]

    @staticmethod
    def marker(phase: str, command_id: str, exit_code: int, source: str, error: str) -> str:
        """Exact allowlisted boundary format emitted by the browser launchers."""
        return (
            f"HEPH_GCP_FAILURE phase={phase} command_id={command_id} exit_code={exit_code} "
            f"diagnostic_source={source} diagnostic_error={error}\n"
        )

    def write_manifest(self, root: Path, *, installed: bool = True) -> str:
        paths = {
            "archive_path": "/usr/share/hephaestus/installed-ui-browser-image.oci",
            "build_path": "/usr/local/libexec/hephaestus/installed-ui-browser-image-build.sh",
            "dockerfile_path": "/usr/local/libexec/hephaestus/installed-ui-browser-image.Dockerfile",
        }
        contents = {
            paths["archive_path"]: b"reviewed OCI archive\n",
            paths["build_path"]: b"#!/usr/bin/env bash\n",
            paths["dockerfile_path"]: b"FROM reviewed\n",
        }
        for relative, content in contents.items():
            target = root / relative.lstrip("/")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
            target.chmod(0o644)
        document: dict[str, object] = {
            "schema": 1,
            "kind": "hephaestus-gcp-runner",
            "required_paths": list(paths.values()) if installed else [],
        }
        if installed:
            document["installed_ui_image"] = {
                **paths,
                "archive_sha256": hashlib.sha256(contents[paths["archive_path"]]).hexdigest(),
                "image_tag": "localhost/hephestus-playwright:1.62.0-certutil",
                "image_digest": "sha256:" + "1" * 64,
                "image_id": "sha256:" + "2" * 64,
                "build_sha256": hashlib.sha256(contents[paths["build_path"]]).hexdigest(),
                "dockerfile_sha256": hashlib.sha256(contents[paths["dockerfile_path"]]).hexdigest(),
                "browser_version": "Chromium 151.0.7922.34",
            }
        canonical = json.dumps(document, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
        fingerprint = hashlib.sha256(canonical).hexdigest()
        document["manifest_sha256"] = fingerprint
        manifest = root / "usr/share/hephaestus/runner-image-manifest.json"
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_text(json.dumps(document, sort_keys=True) + "\n", encoding="utf-8")
        manifest.chmod(0o644)
        return fingerprint

    def run_manifest_validation(self, root: Path, fingerprint: str) -> subprocess.CompletedProcess[str]:
        output_path = root / "workload-env"
        harness = root / "manifest-harness.sh"
        harness.write_text(
            "set -Eeuo pipefail\n"
            "runner_image_manifest_path='/usr/share/hephaestus/runner-image-manifest.json'\n"
            "runner_image_manifest_sha=\"$2\"\n"
            "installed_ui_archive=''\ninstalled_ui_archive_sha=''\n"
            "installed_ui_image_tag=''\ninstalled_ui_image_digest=''\n"
            "installed_ui_image_id=''\ninstalled_ui_build_sha=''\n"
            "installed_ui_dockerfile_sha=''\ninstalled_ui_browser_version=''\n"
            "installed_ui_workload_env=()\n"
            "fail() { printf '%s\\n' \"$*\" >&2; return 1; }\n"
            + self.manifest_function
            + "\nvalidate_runner_image_manifest \"$1\"\n"
            + "for item in \"${installed_ui_workload_env[@]}\"; do printf '%s\\n' \"$item\"; done >\"$3\"\n"
            + "touch \"$1/expensive-work\"\n",
            encoding="utf-8",
        )
        return subprocess.run(
            ["bash", str(harness), str(root), fingerprint, str(output_path)],
            text=True,
            capture_output=True,
            check=False,
        )

    def run_attribution(self, log: str, timing: str, *, wrapper_status: int = 101) -> dict[str, object]:
        with tempfile.TemporaryDirectory(prefix="heph-first-failure-attribution-") as raw:
            root = Path(raw)
            log_path = root / "runtime.log"
            timing_path = root / "timing.jsonl"
            failure_path = root / "first-failure.json"
            log_path.write_text(log, encoding="utf-8")
            timing_path.write_text(timing, encoding="utf-8")
            harness = root / "harness.sh"
            harness.write_text(
                "set -Eeuo pipefail\n"
                "first_failure_path=\"$1\"\n"
                "log_file=\"$2\"\n"
                "runtime_failure_recorded=false\n"
                + self.function
                + "\nrecord_workload_first_failure \"$5\" \"$3\" \"$2\"\n",
                encoding="utf-8",
            )
            result = subprocess.run(
                ["bash", str(harness), str(failure_path), str(log_path), str(timing_path), "unused", str(wrapper_status)],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            return json.loads(failure_path.read_text(encoding="utf-8"))

    def test_actual_workload_exit_overrides_wrapper_and_cleanup(self) -> None:
        log = "\n".join(
            [
                "HEPH_GCP_COOKING event=workload-step operation=cooking-workload phase=cooking stage=cargo-build status=failed exit_code=17",
                "HEPH_GCP_COOKING event=workload-result operation=cooking-workload phase=cooking status=failed exit_code=101",
                "HEPH_GCP_SHELL_FAILURE script=cooking-run component=cooking operation=cooking-cleanup reason=read-failed exit_code=1 line=42",
            ]
        )
        value = self.run_attribution(log, "", wrapper_status=101)
        self.assertEqual(
            value,
            {
                "schema": 1,
                "phase": "cooking",
                "command_id": "cooking-workload",
                "exit_code": 17,
                "diagnostic_source": "runtime-log",
                "diagnostic_error": "phase-failed",
            },
        )

    def test_pre_browser_runtime_image_failure_is_not_browser_image_failure(self) -> None:
        log = "\n".join(
            [
                "HEPH_GCP_COOKING event=image-step phase=workflow-images command=skopeo-copy image=python-ubuntu status=fail exit=23",
                "HEPH_GCP_COOKING event=phase-result phase=workflow-images status=failed exit_code=101",
            ]
        )
        value = self.run_attribution(log, "", wrapper_status=101)
        self.assertEqual(value["phase"], "workflow-images")
        self.assertEqual(value["command_id"], "cooking-workload")
        self.assertEqual(value["exit_code"], 23)
        self.assertEqual(value["diagnostic_source"], "runtime-log")

    def test_installed_ui_image_emitter_failure_wins_later_wrapper_and_cleanup(self) -> None:
        log = "\n".join(
            [
                "HEPH_GCP_COOKING event=installed-ui-image status=build-failed exit_code=17 scan_exit_code=0 log_exit_code=0",
                "HEPH_GCP_COOKING event=workload-result operation=cooking-workload phase=cooking status=failed exit_code=101",
                "HEPH_GCP_SHELL_FAILURE script=cooking-run component=cooking operation=cooking-cleanup reason=read-failed exit_code=1 line=42",
            ]
        )
        value = self.run_attribution(log, "", wrapper_status=101)
        self.assertEqual(
            value,
            {
                "schema": 1,
                "phase": "browser-setup",
                "command_id": "installed-ui-image-build",
                "exit_code": 17,
                "diagnostic_source": "image-build-log",
                "diagnostic_error": "image-build-failed",
            },
        )

    def test_canonical_browser_failure_contract_preserves_npm_and_playwright(self) -> None:
        setup = self.run_attribution(
            self.marker("browser-setup", "browser-setup", 3, "setup-log", "setup-failed"),
            "",
            wrapper_status=101,
        )
        self.assertEqual(setup["phase"], "browser-setup")
        self.assertEqual(setup["command_id"], "browser-setup")
        self.assertEqual(setup["exit_code"], 3)

        npm = self.run_attribution(
            self.marker("browser-setup", "npm-install", 19, "setup-log", "setup-failed"),
            "",
            wrapper_status=101,
        )
        self.assertEqual(npm["phase"], "browser-setup")
        self.assertEqual(npm["command_id"], "npm-install")
        self.assertEqual(npm["exit_code"], 19)
        self.assertEqual(npm["diagnostic_source"], "setup-log")
        self.assertEqual(npm["diagnostic_error"], "setup-failed")

        for phase in ("browser-initial", "browser-recovery", "browser-concurrency", "browser-fork"):
            launcher_npm = self.run_attribution(
                self.marker(phase, "npm-install", 19, "npm-log", "npm-failed"),
                "",
                wrapper_status=101,
            )
            self.assertEqual(launcher_npm["phase"], phase)
            self.assertEqual(launcher_npm["command_id"], "npm-install")
            self.assertEqual(launcher_npm["diagnostic_source"], "npm-log")
            self.assertEqual(launcher_npm["diagnostic_error"], "npm-failed")

        playwright = self.run_attribution(
            self.marker("browser-setup", "playwright-run", 17, "playwright-log", "playwright-failed"),
            json.dumps({"record": "end", "phase": "browser-concurrency", "outcome": "failed"}) + "\n",
            wrapper_status=101,
        )
        self.assertEqual(playwright["phase"], "browser-concurrency")
        self.assertEqual(playwright["command_id"], "playwright-run")
        self.assertEqual(playwright["exit_code"], 17)
        self.assertEqual(playwright["diagnostic_source"], "playwright-log")
        self.assertEqual(playwright["diagnostic_error"], "playwright-failed")

        unknown = self.run_attribution(
            self.marker("browser-setup", "playwright-run", 17, "playwright-log", "playwright-failed"),
            "",
            wrapper_status=101,
        )
        self.assertEqual(unknown["phase"], "unknown")

    def test_browser_typed_exit_overrides_later_wrapper_and_cleanup(self) -> None:
        log = "\n".join(
            [
                "HEPH_BROWSER_BRIDGE event=external-failure status=1 diagnostics-scan=0",
                "HEPH_GCP_COOKING event=workload-result operation=cooking-workload phase=cooking status=failed exit_code=101",
                "HEPH_GCP_SHELL_FAILURE script=cooking-run component=cooking operation=runtime-cleanup reason=read-failed exit_code=1 line=44",
            ]
        )
        timing = json.dumps({"record": "end", "phase": "browser-initial", "outcome": "failed"}) + "\n"
        value = self.run_attribution(log, timing, wrapper_status=101)
        self.assertEqual(value["phase"], "browser-initial")
        self.assertEqual(value["command_id"], "playwright-run")
        self.assertEqual(value["exit_code"], 1)
        self.assertEqual(value["diagnostic_source"], "playwright-log")

        unknown = self.run_attribution(
            "HEPH_BROWSER_BRIDGE event=external-failure status=7 diagnostics-scan=0\n",
            "",
            wrapper_status=101,
        )
        self.assertEqual(unknown["phase"], "unknown")
        self.assertEqual(unknown["command_id"], "playwright-run")
        self.assertEqual(unknown["exit_code"], 7)

    def test_empty_or_malformed_runtime_stream_has_complete_timing_fallback(self) -> None:
        timing = json.dumps({"record": "end", "phase": "browser-recovery", "outcome": "failed"}) + "\n"
        value = self.run_attribution("", timing, wrapper_status=101)
        self.assertEqual(value["phase"], "browser-recovery")
        self.assertEqual(value["command_id"], "playwright-run")
        self.assertEqual(value["exit_code"], 101)

        value = self.run_attribution("not a typed marker\n{malformed", "{malformed", wrapper_status=101)
        self.assertEqual(value["phase"], "cooking-supervisor")
        self.assertEqual(value["command_id"], "cooking-workload")
        self.assertEqual(value["exit_code"], 101)

        oci_timing = json.dumps({"record": "end", "phase": "oci-image-materialization", "outcome": "failed"}) + "\n"
        value = self.run_attribution("", oci_timing, wrapper_status=101)
        self.assertEqual(value["phase"], "oci-image-materialization")
        self.assertEqual(value["command_id"], "cooking-workload")
        self.assertEqual(value["diagnostic_source"], "runtime-log")

    def test_sensitive_or_unallowlisted_markers_are_ignored(self) -> None:
        log = "\n".join(
            [
                "HEPH_GCP_SHELL_FAILURE script=cooking-run component=cooking operation=command reason=command-failed exit_code=17 line=4 token=secret",
                "HEPH_GCP_COOKING event=image-step phase=workflow-images command=skopeo-copy image=python-ubuntu status=fail exit=23 secret=payload",
                "HEPH_GCP_FAILURE phase=browser-setup command_id=npm-install exit_code=17 diagnostic_source=npm-log diagnostic_error=npm-failed token=secret",
                "HEPH_GCP_FAILURE phase=browser-setup command_id=npm-install exit_code=17 diagnostic_source=image-build-log diagnostic_error=image-build-failed",
            ]
        )
        value = self.run_attribution(log, "", wrapper_status=101)
        self.assertEqual(value["phase"], "cooking-supervisor")
        self.assertEqual(value["command_id"], "cooking-workload")
        self.assertEqual(value["exit_code"], 101)
        self.assertNotIn("secret", json.dumps(value))

    def test_valid_baked_archive_metadata_reaches_forge_workload_without_override(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-installed-ui-manifest-") as raw:
            root = Path(raw)
            fingerprint = self.write_manifest(root)
            result = self.run_manifest_validation(root, fingerprint)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            values = (root / "workload-env").read_text(encoding="utf-8").splitlines()
            self.assertIn(
                "--setenv=HEPHAESTUS_PLAYWRIGHT_IMAGE_ARCHIVE=/usr/share/hephaestus/installed-ui-browser-image.oci",
                values,
            )
            self.assertTrue(any(item.startswith("--setenv=HEPHAESTUS_PLAYWRIGHT_IMAGE_ARCHIVE_SHA256=") for item in values))
            self.assertTrue(any(item.startswith("--setenv=HEPH_GCP_INSTALLED_UI_IMAGE_DIGEST=sha256:") for item in values))
            self.assertFalse(any(item.startswith("--setenv=HEPHAESTUS_PLAYWRIGHT_IMAGE=") for item in values))
            systemd_start = self.source.index("timeout --kill-after=30s \"${cooking_remaining}s\" systemd-run")
            systemd_end = self.source.index("/bin/bash -Eeuo pipefail -c", systemd_start)
            self.assertIn('"${installed_ui_workload_env[@]}"', self.source[systemd_start:systemd_end])
            self.assertTrue((root / "expensive-work").is_file())

    def test_old_manifest_without_archive_keeps_source_build_fallback(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-installed-ui-old-manifest-") as raw:
            root = Path(raw)
            fingerprint = self.write_manifest(root, installed=False)
            result = self.run_manifest_validation(root, fingerprint)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual((root / "workload-env").read_text(encoding="utf-8"), "")
            self.assertTrue((root / "expensive-work").is_file())

    def test_malformed_missing_and_tampered_baked_archive_fail_before_workload(self) -> None:
        for failure in ("malformed", "missing", "tampered"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory(prefix="heph-installed-ui-invalid-") as raw:
                root = Path(raw)
                fingerprint = self.write_manifest(root)
                archive = root / "usr/share/hephaestus/installed-ui-browser-image.oci"
                if failure == "malformed":
                    (root / "usr/share/hephaestus/runner-image-manifest.json").write_text("{malformed\n", encoding="utf-8")
                elif failure == "missing":
                    archive.unlink()
                else:
                    archive.write_bytes(b"tampered archive\n")
                result = self.run_manifest_validation(root, fingerprint)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertFalse((root / "expensive-work").exists())


if __name__ == "__main__":
    unittest.main()
