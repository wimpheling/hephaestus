"""Focused tests for safe first-workload-failure attribution."""

from __future__ import annotations

import json
import hashlib
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
SCRIPT = ROOT / "gcp-cooking-run.sh"
COLLECTOR_SPEC = importlib.util.spec_from_file_location(
    "cooking_diagnostics", ROOT / "collect-cooking-diagnostics.py"
)
assert COLLECTOR_SPEC is not None and COLLECTOR_SPEC.loader is not None
COLLECTOR = importlib.util.module_from_spec(COLLECTOR_SPEC)
COLLECTOR_SPEC.loader.exec_module(COLLECTOR)


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

    def write_manifest(
        self,
        root: Path,
        *,
        installed: bool = True,
        browser_version: str = "Chromium 151.0.7922.34",
    ) -> str:
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
                "browser_version": browser_version,
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

    @staticmethod
    def project_with_collector(value: dict[str, object]) -> dict[str, object]:
        with tempfile.TemporaryDirectory(prefix="heph-first-failure-project-") as raw:
            root = Path(raw)
            source = root / "first-failure.json"
            destination = root / "projected-first-failure.json"
            source.write_text(json.dumps(value) + "\n", encoding="utf-8")
            COLLECTOR._project_first_failure(source, destination)
            return json.loads(destination.read_text(encoding="utf-8"))

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
                "phase": "cooking-supervisor",
                "command_id": "cooking-workload",
                "exit_code": 17,
                "diagnostic_source": "runtime-log",
                "diagnostic_error": "phase-failed",
            },
        )

    def test_libkrun_leaf_exit_overrides_golden_wrapper_and_cleanup(self) -> None:
        log = "\n".join(
            [
                "HEPH_GCP_SHELL_FAILURE script=libkrun-integration component=libkrun operation=runtime-cleanup reason=read-failed exit_code=1 line=300",
                "HEPH_GCP_SHELL_FAILURE script=libkrun-integration component=libkrun operation=command reason=command-failed exit_code=101 line=260",
                "HEPH_GCP_COOKING event=workload-step operation=cooking-workload phase=cooking stage=gateway-e2e status=failed exit_code=1",
                "HEPH_GCP_SHELL_FAILURE script=gateway-libkrun-e2e component=gateway operation=command reason=command-failed exit_code=1 line=224",
                "HEPH_GCP_SHELL_FAILURE script=cooking-run component=cooking operation=command reason=process-exit exit_code=1 line=670",
            ]
        )
        timing = json.dumps({"record": "end", "phase": "golden-tests", "outcome": "failed"}) + "\n"
        value = self.run_attribution(log, timing, wrapper_status=1)
        self.assertEqual(
            value,
            {
                "schema": 1,
                "phase": "golden-tests",
                "command_id": "cooking-workload",
                "exit_code": 101,
                "diagnostic_source": "runtime-log",
                "diagnostic_error": "phase-failed",
            },
        )
        self.assertEqual(self.project_with_collector(value), value)

    def test_libkrun_leaf_marker_is_ignored_when_malformed_or_unallowlisted(self) -> None:
        log = "\n".join(
            [
                "HEPH_GCP_SHELL_FAILURE script=libkrun-integration component=libkrun operation=command reason=command-failed exit_code=0 line=260",
                "HEPH_GCP_SHELL_FAILURE script=unknown component=libkrun operation=command reason=command-failed exit_code=101 line=260",
            ]
        )
        value = self.run_attribution(log, "", wrapper_status=1)
        self.assertEqual(value["phase"], "cooking-supervisor")
        self.assertEqual(value["command_id"], "cooking-workload")
        self.assertEqual(value["exit_code"], 1)

    def test_libkrun_leaf_does_not_displace_earlier_oci_or_later_browser_failure(self) -> None:
        oci_log = "\n".join(
            [
                "HEPH_GCP_COOKING event=image-step phase=workflow-images command=skopeo-copy image=python-ubuntu status=fail exit=23",
                "HEPH_GCP_SHELL_FAILURE script=libkrun-integration component=libkrun operation=command reason=command-failed exit_code=101 line=260",
            ]
        )
        oci = self.run_attribution(oci_log, "", wrapper_status=1)
        self.assertEqual(oci["phase"], "workflow-images")
        self.assertEqual(oci["exit_code"], 23)

        browser_log = "\n".join(
            [
                "HEPH_GCP_SHELL_FAILURE script=libkrun-integration component=libkrun operation=command reason=command-failed exit_code=101 line=260",
                self.marker("browser-initial", "playwright-run", 17, "playwright-log", "playwright-failed"),
            ]
        )
        browser_timing = json.dumps({"record": "end", "phase": "browser-initial", "outcome": "failed"}) + "\n"
        browser = self.run_attribution(browser_log, browser_timing, wrapper_status=1)
        self.assertEqual(browser["phase"], "browser-initial")
        self.assertEqual(browser["command_id"], "playwright-run")
        self.assertEqual(browser["exit_code"], 17)

    def test_workload_failure_uses_first_timing_phase_and_projects(self) -> None:
        log = (
            "HEPH_GCP_COOKING event=workload-step operation=cooking-workload "
            "phase=cooking stage=gateway-e2e status=failed exit_code=17\n"
        )
        without_timing = self.run_attribution(log, "", wrapper_status=101)
        self.assertEqual(without_timing["phase"], "cooking-supervisor")
        self.assertEqual(without_timing["command_id"], "cooking-workload")
        self.assertEqual(without_timing["exit_code"], 17)
        self.assertEqual(self.project_with_collector(without_timing), without_timing)

        timing = json.dumps({"record": "end", "phase": "golden-tests", "outcome": "failed"}) + "\n"
        with_timing = self.run_attribution(log, timing, wrapper_status=101)
        self.assertEqual(with_timing["phase"], "golden-tests")
        self.assertEqual(with_timing["command_id"], "cooking-workload")
        self.assertEqual(with_timing["exit_code"], 17)
        self.assertEqual(self.project_with_collector(with_timing), with_timing)

    def test_browser_report_failure_becomes_evidence_first_failure_after_workload_pass(self) -> None:
        log = "\n".join(
            [
                "HEPH_GCP_COOKING event=workload-result operation=cooking-workload phase=cooking status=passed exit_code=0",
                "HEPH_GCP_COOKING event=browser-report-validation operation=browser-report-validation phase=evidence status=failed report_state=missing reason=invalid-report exit_code=2",
                "HEPH_GCP_COOKING event=timing-helper-error operation=cooking-workload phase=cooking stage=timing-helper-validation status=failed exit_code=2 reason_class=pair",
            ]
        )
        value = self.run_attribution(log, "", wrapper_status=2)
        self.assertEqual(
            value,
            {
                "schema": 1,
                "phase": "evidence-scan",
                "command_id": "browser-report-validation",
                "exit_code": 2,
                "diagnostic_source": "runtime-log",
                "diagnostic_error": "phase-failed",
            },
        )
        self.assertEqual(self.project_with_collector(value), value)

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

    def test_browser_leaf_failure_wins_outer_golden_timing_failure(self) -> None:
        marker = self.marker(
            "browser-setup", "playwright-run", 1, "playwright-log", "playwright-failed"
        )
        # Exercise the actual ordering where the outer golden wrapper is
        # logged before the launcher boundary is flushed.
        log = (
            "HEPH_GCP_COOKING event=workload-step operation=cooking-workload "
            "phase=cooking stage=gateway-e2e status=failed exit_code=101\n"
            + marker
            + "HEPH_GCP_COOKING event=browser-report-validation operation=browser-report-validation "
            "phase=evidence status=failed report_state=missing reason=invalid-report exit_code=2\n"
        )
        timing = "".join(
            json.dumps({"record": "end", "phase": phase, "outcome": "failed"}) + "\n"
            for phase in ("golden-tests", "browser-initial")
        )
        value = self.run_attribution(log, timing, wrapper_status=101)
        self.assertEqual(
            value,
            {
                "schema": 1,
                "phase": "browser-initial",
                "command_id": "playwright-run",
                "exit_code": 1,
                "diagnostic_source": "playwright-log",
                "diagnostic_error": "playwright-failed",
            },
        )
        self.assertEqual(self.project_with_collector(value), value)

    def test_retained_bare_browser_boundary_wins_golden_wrapper(self) -> None:
        # This is the shape retained by Cooking 35775999828: the launcher
        # emitted a minimal phase/exit boundary and the outer golden wrapper
        # was also reported as failed.  The previous parser ignored that
        # bounded browser marker.
        log = "\n".join(
            [
                "HEPH_GCP_COOKING event=workload-step operation=cooking-workload "
                "phase=cooking stage=gateway-e2e status=failed exit_code=101",
                "HEPH_BROWSER_BRIDGE event=external-failure status=1 diagnostics-scan=0",
                "HEPH_GCP_FAILURE phase=browser-initial exit_code=1",
            ]
        ) + "\n"
        timing = "".join(
            json.dumps({"record": "end", "phase": phase, "outcome": "failed"}) + "\n"
            for phase in ("golden-tests", "browser-initial")
        )
        value = self.run_attribution(log, timing, wrapper_status=101)
        self.assertEqual(
            value,
            {
                "schema": 1,
                "phase": "browser-initial",
                "command_id": "playwright-run",
                "exit_code": 1,
                "diagnostic_source": "playwright-log",
                "diagnostic_error": "playwright-failed",
            },
        )
        self.assertEqual(self.project_with_collector(value), value)

    def test_retained_bare_browser_boundary_does_not_displace_production_failure(self) -> None:
        log = (
            "HEPH_GCP_COOKING event=workload-step operation=cooking-workload "
            "phase=cooking stage=project-build status=failed exit_code=101\n"
            "HEPH_GCP_FAILURE phase=browser-initial exit_code=1\n"
        )
        timing = "".join(
            json.dumps({"record": "end", "phase": phase, "outcome": "failed"}) + "\n"
            for phase in ("production-project-build", "browser-initial")
        )
        value = self.run_attribution(log, timing, wrapper_status=101)
        self.assertEqual(value["phase"], "production-project-build")
        self.assertEqual(value["command_id"], "cooking-workload")
        self.assertEqual(value["exit_code"], 101)

    def test_bare_browser_boundary_requires_known_phase_and_exit(self) -> None:
        log = "\n".join(
            [
                "HEPH_GCP_FAILURE phase=browser-unknown exit_code=1",
                "HEPH_GCP_FAILURE phase=browser-initial exit_code=0",
                "HEPH_GCP_FAILURE phase=browser-initial exit_code=256",
            ]
        ) + "\n"
        value = self.run_attribution(log, "", wrapper_status=101)
        self.assertEqual(value["phase"], "cooking-supervisor")
        self.assertEqual(value["command_id"], "cooking-workload")
        self.assertEqual(value["exit_code"], 101)

    def test_external_failure_with_golden_timing_uses_safe_workload_contract(self) -> None:
        log = "HEPH_BROWSER_BRIDGE event=external-failure status=101 diagnostics-scan=0\n"
        timing = json.dumps({"record": "end", "phase": "golden-tests", "outcome": "failed"}) + "\n"
        value = self.run_attribution(log, timing, wrapper_status=101)
        self.assertEqual(value["phase"], "golden-tests")
        self.assertEqual(value["command_id"], "cooking-workload")
        self.assertEqual(value["exit_code"], 101)
        self.assertEqual(value["diagnostic_source"], "runtime-log")
        self.assertEqual(value["diagnostic_error"], "phase-failed")
        self.assertEqual(self.project_with_collector(value), value)

    def test_external_failure_accepts_every_valid_workload_timing_phase(self) -> None:
        phases = (
            "dependency-setup", "cache-download", "cache-extract", "workflow-images",
            "browser-setup", "metadata-guard", "project-build", "production-project-build",
            "runtime-guest-build", "runtime-worker-build", "runtime-smoke", "gateway-edge-ready",
            "gateway-services-ready", "gateway-readiness", "oci-image-materialization", "oci-builder",
            "oci-verifier", "golden-tests", "database-tests", "browser-initial", "browser-recovery",
            "browser-concurrency", "browser-fork", "guest-negative-capability", "browser-post-operation",
        )
        browser_contracts = {
            "browser-setup": ("browser-setup", "setup-log", "setup-failed"),
            "browser-initial": ("playwright-run", "playwright-log", "playwright-failed"),
            "browser-recovery": ("playwright-run", "playwright-log", "playwright-failed"),
            "browser-concurrency": ("playwright-run", "playwright-log", "playwright-failed"),
            "browser-fork": ("playwright-run", "playwright-log", "playwright-failed"),
            "browser-post-operation": ("playwright-run", "playwright-log", "playwright-failed"),
        }
        for phase in phases:
            with self.subTest(phase=phase):
                timing = json.dumps({"record": "end", "phase": phase, "outcome": "failed"}) + "\n"
                value = self.run_attribution(
                    "HEPH_BROWSER_BRIDGE event=external-failure status=101 diagnostics-scan=0\n",
                    timing,
                    wrapper_status=101,
                )
                self.assertEqual(value["phase"], phase)
                expected_command, expected_source, expected_error = browser_contracts.get(
                    phase, ("cooking-workload", "runtime-log", "phase-failed")
                )
                self.assertEqual(value["command_id"], expected_command)
                self.assertEqual(value["diagnostic_source"], expected_source)
                self.assertEqual(value["diagnostic_error"], expected_error)
                self.assertEqual(self.project_with_collector(value), value)

    def test_browser_post_operation_typed_exit_wins_wrapper_and_roundtrips(self) -> None:
        marker = self.marker(
            "browser-post-operation", "playwright-run", 17, "playwright-log", "playwright-failed"
        )
        log = (
            "HEPH_GCP_COOKING event=workload-step operation=cooking-workload "
            "phase=cooking stage=gateway-e2e status=failed exit_code=101\n"
            + marker
        )
        timing = "".join(
            json.dumps({"record": "end", "phase": phase, "outcome": "failed"}) + "\n"
            for phase in ("golden-tests", "browser-post-operation")
        )
        value = self.run_attribution(log, timing, wrapper_status=101)
        self.assertEqual(
            value,
            {
                "schema": 1,
                "phase": "browser-post-operation",
                "command_id": "playwright-run",
                "exit_code": 17,
                "diagnostic_source": "playwright-log",
                "diagnostic_error": "playwright-failed",
            },
        )
        self.assertEqual(self.project_with_collector(value), value)

    def test_earlier_production_failure_remains_primary_over_later_browser(self) -> None:
        timing = "".join(
            json.dumps({"record": "end", "phase": phase, "outcome": "failed"}) + "\n"
            for phase in ("production-project-build", "browser-initial")
        )
        value = self.run_attribution(
            "HEPH_BROWSER_BRIDGE event=external-failure status=101 diagnostics-scan=0\n",
            timing,
            wrapper_status=101,
        )
        self.assertEqual(value["phase"], "production-project-build")
        self.assertEqual(value["command_id"], "cooking-workload")
        self.assertEqual(value["exit_code"], 101)

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

    def test_cooking_scenario_keeps_full_build_proof_path(self) -> None:
        cooking_start = self.source.index('    workload_scenario_env=(\n        "--setenv=HEPHAESTUS_APP_COOKING_E2E=1"')
        cooking_end = self.source.index(
            "    printf 'HEPH_GCP_COOKING event=scenario-selected scenario=cooking\\n'",
            cooking_start,
        )
        cooking_branch = self.source[cooking_start:cooking_end]
        self.assertIn(
            '"--setenv=HEPHAESTUS_APP_COOKING_BUILD_PROOF=1"',
            cooking_branch,
        )
        self.assertNotIn(
            '"--setenv=HEPHAESTUS_APP_COOKING_SERVICE_BUILD_PROOF=1"',
            cooking_branch,
        )

        session_start = self.source.index('    workload_scenario_env=(\n        "--setenv=HEPHAESTUS_APP_SESSION_CHAT_E2E=1"')
        session_end = self.source.index(
            "    printf 'HEPH_GCP_COOKING event=scenario-selected scenario=session-chat\\n'",
            session_start,
        )
        session_branch = self.source[session_start:session_end]
        self.assertNotIn(
            '"--setenv=HEPHAESTUS_APP_COOKING_SERVICE_BUILD_PROOF=1"',
            session_branch,
        )

    def test_reviewed_chrome_for_testing_metadata_reaches_forge_workload(self) -> None:
        """Exercise the production validator with the baked candidate's browser identity."""
        for browser_version in (
            "Chromium 151.0.7922.34",
            "Google Chrome for Testing 151.0.7922.34",
        ):
            with self.subTest(browser_version=browser_version), tempfile.TemporaryDirectory(
                prefix="heph-installed-ui-browser-version-"
            ) as raw:
                root = Path(raw)
                fingerprint = self.write_manifest(root, browser_version=browser_version)
                result = self.run_manifest_validation(root, fingerprint)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                values = (root / "workload-env").read_text(encoding="utf-8").splitlines()
                self.assertIn(
                    f"--setenv=HEPH_GCP_INSTALLED_UI_BROWSER_VERSION={browser_version}",
                    values,
                )

    def test_unreviewed_browser_version_remains_rejected_by_production_validator(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-installed-ui-browser-version-invalid-") as raw:
            root = Path(raw)
            fingerprint = self.write_manifest(
                root,
                browser_version="Google Chrome for Testing 151.0.7922.35",
            )
            result = self.run_manifest_validation(root, fingerprint)
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse((root / "expensive-work").exists())

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
