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
        self.assertLess(
            startup.index("phase_start runner-image-runtime"),
            startup.index("phase_start diagnostic-bootstrap"),
        )
        self.assertLess(
            startup.index("metadata_value diagnostics-collector-script"),
            startup.index("runner_image_ready=false"),
        )
        self.assertIn("stage_diagnostics_metadata", startup)
        self.assertLess(
            startup.index("stage_diagnostics_metadata\n"),
            startup.index("runner_image_ready=false"),
        )
        self.assertIn("llvm_prefix=\"$(llvm-config --prefix)\"", startup)
        self.assertIn("-name 'libclang.so*'", startup)
        subprocess.run(["bash", "-n", str(SCRIPT.with_name("gcp-kvm-startup.sh"))], check=True)

    def test_custom_runtime_tools_run_as_forge_and_fail_on_version_mismatch(self) -> None:
        startup = SCRIPT.with_name("gcp-kvm-startup.sh")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            browser_root = root / "srv/hephaestus/playwright-browsers"
            browser_root.mkdir(parents=True)
            browser = browser_root / "chrome-headless-shell"
            browser.write_text(
                "#!/bin/sh\n"
                "case \"$1\" in\n"
                "  --version) printf '%s\\n' 'Chromium 1.2.3' ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            browser.chmod(0o555)
            for relative, body in {
                "usr/local/bin/node": "#!/bin/sh\nprintf '%s\\n' 'v24.16.0'\n",
                "usr/local/bin/oras": "#!/bin/sh\nprintf '%s\\n' 'Version: 1.3.3'\n",
                "home/forge/.cargo/bin/rustc": "#!/bin/sh\nprintf '%s\\n' 'rustc 1.88.0 (baked)'\n",
            }.items():
                tool = root / relative
                tool.parent.mkdir(parents=True, exist_ok=True)
                tool.write_text(body, encoding="utf-8")
                tool.chmod(0o555)
            runuser = root / "runuser"
            runuser.write_text(
                "#!/bin/sh\n"
                "while [ \"$1\" != -- ]; do shift; done\n"
                "shift\n"
                "exec /usr/bin/env \"$@\"\n",
                encoding="utf-8",
            )
            runuser.chmod(0o555)
            test_env = {
                **os.environ,
                "HEPH_GCP_STARTUP_LIBRARY": "1",
                "HEPH_GCP_RUNNER_IMAGE_RUNTIME_TEST": "1",
                "HEPH_GCP_RUNNER_IMAGE_TEST_ROOT": str(root),
                "HEPH_GCP_RUNNER_IMAGE_TEST_BROWSER_ROOT": str(browser_root),
                "PATH": f"{root}:/usr/local/bin:/usr/bin:/bin",
            }
            command = ["bash", "-c", 'source "$1"', "runner-image-runtime-test", str(startup)]
            result = subprocess.run(command, env=test_env, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("runtime=pass", result.stdout)
            browser.chmod(0o644)
            browser.write_text(browser.read_text(encoding="utf-8").replace("Chromium 1.2.3", "Chromium 9.9.9"), encoding="utf-8")
            browser.chmod(0o555)
            result = subprocess.run(command, env=test_env, text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Chromium executable version does not match", result.stderr)
            browser.chmod(0o444)
            result = subprocess.run(command, env=test_env, text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Chromium executable is missing", result.stderr)

    def test_early_image_failure_can_stage_metadata_collector_before_finish(self) -> None:
        startup = SCRIPT.with_name("gcp-kvm-startup.sh")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            metadata_root = root / "metadata"
            collector = root / "collector.py"
            scanner = root / "scanner.py"
            collector.write_text("#!/usr/bin/env python3\n", encoding="utf-8")
            scanner.write_text("#!/usr/bin/env python3\n", encoding="utf-8")
            command = r'''
source "$1"
test_mode=diagnostic
trap - EXIT
collector_path="$3"
scanner_path="$4"
metadata_value() {
  case "$1" in
    diagnostics-collector-script) cat "$collector_path" ;;
    diagnostics-scanner-script) cat "$scanner_path" ;;
    *) return 1 ;;
  esac
}
stage_diagnostics_metadata
test -s "$diagnostics_metadata_root/collect-cooking-diagnostics.py"
test -s "$diagnostics_metadata_root/check-browser-evidence.py"
'''
            result = subprocess.run(
                [
                    "bash", "-Eeuo", "pipefail", "-c", command, "metadata-stage-test",
                    str(startup), str(metadata_root), str(collector), str(scanner),
                ],
                env={
                    **os.environ,
                    "HEPH_GCP_STARTUP_LIBRARY": "1",
                    "HEPH_GCP_DIAGNOSTICS_METADATA_ROOT": str(metadata_root),
                },
                text=True,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                (metadata_root / "collect-cooking-diagnostics.py").read_text(encoding="utf-8"),
                collector.read_text(encoding="utf-8"),
            )

    def test_early_image_failure_finish_collects_with_staged_metadata(self) -> None:
        startup = SCRIPT.with_name("gcp-kvm-startup.sh")
        collector_source = SCRIPT.with_name("collect-cooking-diagnostics.py")
        scanner_source = SCRIPT.with_name("check-browser-evidence.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            work = root / "work"
            metadata_root = root / "metadata"
            metadata_root.mkdir(parents=True)
            collector = root / "collector.py"
            scanner = root / "scanner.py"
            collector.write_bytes(collector_source.read_bytes())
            scanner.write_bytes(scanner_source.read_bytes())
            log_file = root / "startup.log"
            log_file.write_text("runner image manifest verification failed\n", encoding="utf-8")
            fake_curl = root / "curl"
            fake_curl.write_text(
                "#!/bin/sh\n"
                "case \"$*\" in\n"
                "  *instance/service-accounts/default/token*) printf '%s' '{\"access_token\":\"test-token\"}' ;;\n"
                "  *storage.googleapis.com/upload*) exit 0 ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            fake_curl.chmod(0o755)
            command = r'''
source "$1"
trap - EXIT
collector_path="$3"
scanner_path="$4"
metadata_value() {
  case "$1" in
    diagnostics-collector-script) cat "$collector_path" ;;
    diagnostics-scanner-script) cat "$scanner_path" ;;
    diagnostics-object) printf '%s\n' 'cooking/runs/1/1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.tar.gz' ;;
    *) return 1 ;;
  esac
}
test_mode=diagnostic
phase=runner-image-verify
revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
vm_start_epoch=$(date +%s)
collection_deadline_epoch=$((vm_start_epoch + 60))
stage_diagnostics_metadata
set +e
(exit 17)
finish
'''
            env = {
                **os.environ,
                "HEPH_GCP_STARTUP_LIBRARY": "1",
                "HEPH_GCP_WORK_ROOT": str(work),
                "HEPH_GCP_LOG_FILE": str(log_file),
                "HEPH_GCP_DIAGNOSTICS_METADATA_ROOT": str(metadata_root),
                "PATH": f"{root}:{os.environ['PATH']}",
            }
            result = subprocess.run(
                [
                    "bash", "-Eeuo", "pipefail", "-c", command, "early-image-failure-test",
                    str(startup), str(metadata_root), str(collector), str(scanner),
                ],
                env=env,
                text=True,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 17, result.stdout + result.stderr)
            self.assertIn("event=upload status=pass", result.stdout)
            self.assertIn("HEPHAESTUS_GCP_DIAGNOSTIC: TEST-FAIL expected=false", result.stdout)
            self.assertTrue((work / "tmp/cooking-diagnostics.tar.gz").is_file())

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

    def test_bake_normalizes_copied_node_tree_for_nonroot_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stage = root / "stage"
            destination = root / "node-v24.16.0"
            (stage / "bin").mkdir(parents=True)
            (stage / "share").mkdir()
            (stage / "bin/node").write_text("#!/bin/sh\n", encoding="utf-8")
            (stage / "bin/node").chmod(0o755)
            (stage / "share/LICENSE").write_text("license\n", encoding="utf-8")
            stage.chmod(0o700)
            destination.parent.mkdir(exist_ok=True)
            subprocess.run(
                ["cp", "-a", "--no-preserve=ownership", str(stage), str(destination)],
                check=True,
            )
            result = subprocess.run(
                ["bash", str(Path(__file__).with_name("gcp-runner-image-bake.sh")), str(destination)],
                env={**os.environ, "HEPH_GCP_IMAGE_PERMISSION_TEST": "1"},
                text=True,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(destination.stat().st_mode & 0o777, 0o555)
            self.assertEqual((destination / "bin").stat().st_mode & 0o777, 0o555)
            self.assertEqual((destination / "bin/node").stat().st_mode & 0o777, 0o555)
            self.assertEqual((destination / "share/LICENSE").stat().st_mode & 0o777, 0o444)
            self.assertTrue(os.access(destination / "bin/node", os.X_OK))

    def test_bake_generalizes_identity_and_preserves_nextboot_services(self) -> None:
        bake = Path(__file__).with_name("gcp-runner-image-bake.sh")
        bake_text = bake.read_text(encoding="utf-8")
        startup_call = bake_text.index('bash "$script_dir/gcp-kvm-startup.sh"')
        marker_install = bake_text.index('install -m 0644 /dev/null /etc/hephaestus/runner-image-required')
        self.assertGreater(marker_install, startup_call)
        self.assertLess(
            bake_text.rindex("generalize_runner_image"),
            bake_text.index("HEPH_GCP_RUNNER_IMAGE: READY"),
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in (
                "etc/ssh",
                "var/lib/cloud/instances/i-1",
                "var/lib/cloud/instance",
                "var/lib/cloud/sem",
                "var/lib/cloud/data",
                "var/lib/cloud/seed",
                "var/lib/dbus",
                "var/lib/google",
                "var/log",
            ):
                (root / relative).mkdir(parents=True, exist_ok=True)
            for relative in (
                "etc/ssh/ssh_host_ed25519_key",
                "etc/ssh/ssh_host_ed25519_key.pub",
                "etc/google_instance_id",
                "var/lib/dbus/machine-id",
                "var/lib/google/instance_id",
                "var/log/cloud-init.log",
                "var/log/cloud-init-output.log",
            ):
                target = root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text("sensitive identity state\n", encoding="utf-8")
            machine_id = root / "etc/machine-id"
            machine_id.parent.mkdir(parents=True, exist_ok=True)
            machine_id.write_text("0123456789abcdef\n", encoding="utf-8")
            cloud_init = root / "cloud-init"
            cloud_init.write_text(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >\"$HEPH_IMAGE_TEST_CLOUD_INIT_LOG\"\n",
                encoding="utf-8",
            )
            cloud_init.chmod(0o755)
            systemctl = root / "systemctl"
            systemctl.write_text(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >>\"$HEPH_IMAGE_TEST_SYSTEMCTL_LOG\"\n",
                encoding="utf-8",
            )
            systemctl.chmod(0o755)
            test_env = {
                **os.environ,
                "HEPH_GCP_IMAGE_GENERALIZE_TEST": "1",
                "HEPH_IMAGE_TEST_CLOUD_INIT_LOG": str(root / "cloud-init.args"),
                "HEPH_IMAGE_TEST_SYSTEMCTL_LOG": str(root / "systemctl.args"),
                "PATH": f"{root}:/usr/local/bin:/usr/bin:/bin",
            }
            result = subprocess.run(
                ["bash", str(bake), str(root)], env=test_env, text=True, capture_output=True
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            for relative in (
                "etc/ssh/ssh_host_ed25519_key",
                "etc/ssh/ssh_host_ed25519_key.pub",
                "etc/google_instance_id",
                "var/lib/dbus/machine-id",
                "var/lib/google/instance_id",
                "var/log/cloud-init.log",
                "var/log/cloud-init-output.log",
            ):
                self.assertFalse((root / relative).exists(), relative)
            self.assertEqual(machine_id.read_text(encoding="utf-8"), "")
            self.assertFalse((root / "var/lib/cloud/instances").exists())
            self.assertEqual((root / "cloud-init.args").read_text(encoding="utf-8"), "clean --logs --seed\n")
            enabled = (root / "systemctl.args").read_text(encoding="utf-8")
            self.assertIn("--root=" + str(root) + " enable google-guest-agent.service google-startup-scripts.service", enabled)
            self.assertNotRegex(enabled, r"\b(start|stop|disable)\b")


if __name__ == "__main__":
    unittest.main()
