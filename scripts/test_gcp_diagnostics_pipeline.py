"""Focused regression tests for the private GCP diagnostics download gate."""

from __future__ import annotations

import importlib.util
import hashlib
import io
import json
import os
from pathlib import Path
import pwd
import stat
import subprocess
import tempfile
import tarfile
import unittest
import shutil


ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location(
    "cooking_diagnostics", ROOT / "collect-cooking-diagnostics.py"
)
COLLECTOR = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(COLLECTOR)
TRIAGE_SPEC = importlib.util.spec_from_file_location(
    "cooking_diagnostics_triage", ROOT / "summarize-cooking-diagnostics.py"
)
TRIAGE = importlib.util.module_from_spec(TRIAGE_SPEC)
assert TRIAGE_SPEC.loader is not None
TRIAGE_SPEC.loader.exec_module(TRIAGE)
PROJECTOR_SPEC = importlib.util.spec_from_file_location(
    "playwright_browser_summary", ROOT / "project-playwright-browser-summary.py"
)
PROJECTOR = importlib.util.module_from_spec(PROJECTOR_SPEC)
assert PROJECTOR_SPEC.loader is not None
PROJECTOR_SPEC.loader.exec_module(PROJECTOR)


class GcpDiagnosticsPipelineTests(unittest.TestCase):
    def _run_real_finish(
        self,
        mode: str,
        upload: str = "ok",
        scanner: str = "ok",
        workload: str = "success",
    ) -> tuple[subprocess.CompletedProcess[str], Path]:
        root = Path(tempfile.mkdtemp(prefix="heph-gcp-finish-"))
        work = root / "work"
        metadata_root = root / "metadata"
        metadata_root.mkdir(parents=True)
        checkout_scripts = work / "checkout" / "scripts"
        checkout_scripts.mkdir(parents=True)
        shutil.copy2(ROOT / "collect-cooking-diagnostics.py", checkout_scripts / "collect-cooking-diagnostics.py")
        shutil.copy2(ROOT / "check-browser-evidence.py", checkout_scripts / "check-browser-evidence.py")
        shutil.copy2(ROOT / "collect-cooking-diagnostics.py", metadata_root / "collect-cooking-diagnostics.py")
        shutil.copy2(ROOT / "check-browser-evidence.py", metadata_root / "check-browser-evidence.py")
        if scanner == "reject":
            shutil.copy2(ROOT / "check-browser-evidence.py", checkout_scripts / "check-browser-evidence-real.py")
            (checkout_scripts / "check-browser-evidence.py").write_text(
                "#!/usr/bin/env python3\n"
                "import importlib.util\n"
                "spec=importlib.util.spec_from_file_location('real', __file__.replace('check-browser-evidence.py', 'check-browser-evidence-real.py'))\n"
                "real=importlib.util.module_from_spec(spec); spec.loader.exec_module(real)\n"
                "main=real.main\n"
                "if __name__ == '__main__': raise SystemExit(7)\n",
                encoding="utf-8",
            )
            shutil.copy2(checkout_scripts / "check-browser-evidence.py", metadata_root / "check-browser-evidence.py")
        evidence = work / "evidence" / "cooking"
        evidence.mkdir(parents=True, mode=0o700)
        (evidence / "cooking-lineage.jsonl").write_text("", encoding="utf-8")
        (evidence / "cooking-lineage-status.json").write_text(
            '{"schema":1,"status":"ok","sampled_at":"2026-09-11 10:00:00 Z",'
            '"mailbox_id":"00000000-0000-4000-8000-000000000001","event_id":null,"rows":0}\n',
            encoding="utf-8",
        )
        (evidence / "browser-summary.json").write_text(
            '{"status":"passed","suite":"cooking-playwright","test":"browser-journey",'
            '"phase":"browser","exit_code":0}\n', encoding="utf-8"
        )
        log_file = root / "startup.log"
        log_file.write_text("HEPH_GCP_KVM_STARTUP event=phase-pass phase=running revision=" + "a" * 40 + "\n", encoding="utf-8")
        fake_bin = root / "bin"
        fake_bin.mkdir()
        (fake_bin / "curl").write_text(
            "#!/usr/bin/env bash\nset -Eeuo pipefail\n"
            "case \"$*\" in\n"
            "  *instance/service-accounts/default/token*) printf '{\"access_token\":\"test-token\"}';;\n"
            f"  *storage.googleapis.com/upload*) case \"${{HEPH_FAKE_UPLOAD:-ok}}\" in 403) exit 22;; hang) sleep 10;; esac;;\n"
            "  *) exit 2;;\n"
            "esac\n",
            encoding="utf-8",
        )
        (fake_bin / "curl").chmod(0o700)
        command = r'''
source "$1"
test_mode="$2"
phase=running
revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
diagnostics_object_metadata='cooking/runs/1/1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.tar.gz'
vm_start_epoch=$(date +%s)
collection_deadline_epoch=$((vm_start_epoch + ${HEPH_FAKE_DEADLINE:-30}))
if [[ "$test_mode" == diagnostic ]]; then
  phase=diagnostic-synthetic
  diagnostic_probe_completed=true
  set +e
  (exit 42)
else
  set +e
  case "${HEPH_FINISH_RESULT:-success}" in
    failure) (exit 7) ;;
    timeout) timeout --kill-after=1s 1s bash -c 'sleep 30' ;;
    *) true ;;
  esac
fi
finish
'''
        env = os.environ | {
            "HEPH_GCP_STARTUP_LIBRARY": "1",
            "HEPH_GCP_WORK_ROOT": str(work),
            "HEPH_GCP_LOG_FILE": str(log_file),
            "HEPH_GCP_DIAGNOSTICS_METADATA_ROOT": str(metadata_root),
            "HEPH_FAKE_UPLOAD": upload,
            "HEPH_FAKE_DEADLINE": "2" if upload == "hang" else "30",
            "HEPH_FINISH_RESULT": workload,
            "PATH": f"{fake_bin}:{os.environ['PATH']}",
        }
        result = subprocess.run(
            ["bash", "-Eeuo", "pipefail", "-c", command, "finish-test", str(ROOT / "gcp-kvm-startup.sh"), mode],
            env=env, text=True, capture_output=True, check=False,
        )
        return result, root

    def _archive(self, root: Path, gate_results: Path | None = None) -> Path:
        source_root = root / "sources"
        source_root.mkdir()
        files = {
            "serial.log": "HEPH_GCP_DIAGNOSTIC: TEST-FAIL expected=true\n",
            "host-journal.log": "HEPH_GCP_DIAGNOSTICS status=unavailable\n",
            "runtime-structured.json": (
                "HEPH_GCP_DIAGNOSTIC test_result=42 diagnostics_result=ready\n"
                "HEPHAESTUS_RUNTIME denial_stage=session-authentication "
                "denial_class=authentication_denied "
                "run_id=00000000-0000-4000-8000-000000000004\n"
                "HEPHAESTUS_RUNTIME status=truncated\n"
            ),
            "browser-summary.json": '{"status":"failed","phase":"browser","test":"diagnostic-synthetic","exit_code":42}\n',
            "test-output.log": "expected diagnostic failure\n",
        }
        for name, content in files.items():
            (source_root / name).write_text(content, encoding="utf-8")
        lineage = source_root / "lineage.jsonl"
        lineage.write_text(
            '{"attempt_id":"00000000-0000-4000-8000-000000000001",'
            '"attempt_number":1,"attempt_run_id":"00000000-0000-4000-8000-000000000004",'
            '"attempt_state":"failed",'
            '"run_state":"failed","run_outcome":"failed"}\n',
            encoding="utf-8",
        )
        output = root / "bundle"
        archive = root / "bundle.tar.gz"
        args = ["--output-dir", str(output)]
        for name in files:
            label = name.removesuffix(".json").removesuffix(".log")
            args += ["--source", f"{label}={source_root / name}"]
        if gate_results is not None:
            args += ["--source", f"gate-results={gate_results}"]
        args += ["--snapshot-jsonl", str(lineage), "--archive", str(archive)]
        self.assertEqual(COLLECTOR.main(args), 0)
        return archive

    def _cooking_gate_results(self, root: Path, *, workload_exit: int = 0) -> Path:
        helper = ROOT / "cooking-gate-results.py"
        state = "passed" if workload_exit == 0 else "failed"
        reason = "none" if workload_exit == 0 else "workload-failed"
        gate_results = root / "gate-results.json"
        value = {
            "schema": 1,
            "revision": "a" * 40,
            "script_sha256": hashlib.sha256(helper.read_bytes()).hexdigest(),
            "test_mode": "gcp-cooking",
            "overall_exit_code": workload_exit,
            "supervisor_exit_code": workload_exit,
            "finalized": True,
            "gates": {
                "workload": {"state": state, "exit_code": workload_exit, "reason_class": reason},
                "evidence-scan": {"state": "passed", "exit_code": 0, "reason_class": "none"},
                "browser-validation": {"state": "passed", "exit_code": 0, "reason_class": "none"},
            },
        }
        gate_results.write_text(json.dumps(value) + "\n", encoding="utf-8")
        return gate_results

    def test_failed_gate_sidecar_is_triaged_before_acceptance_fails(self):
        """A valid failed Cooking sidecar retains triage evidence and fails the gate."""

        with tempfile.TemporaryDirectory(prefix="heph-gcp-failed-gate-") as directory:
            root = Path(directory)
            sidecar = root / "gate-results.json"
            helper = ROOT / "cooking-gate-results.py"
            helper_args = ["--path", str(sidecar), "--script-file", str(helper)]
            commands = [
                helper_args + [
                    "init", "--revision", "a" * 40,
                    "--script-sha256", hashlib.sha256(helper.read_bytes()).hexdigest(),
                    "--test-mode", "gcp-cooking",
                ],
            ]
            for gate, state, code, reason in (
                ("workload", "failed", "7", "workload-failed"),
                ("evidence-scan", "passed", "0", "none"),
                ("browser-validation", "passed", "0", "none"),
            ):
                commands.extend([
                    helper_args + ["begin", gate],
                    helper_args + ["complete", gate, "--state", state, "--exit-code", code, "--reason-class", reason],
                ])
            commands.append(helper_args + [
                "finalize", "--overall-exit-code", "7", "--supervisor-exit-code", "7",
            ])
            for command in commands:
                self.assertEqual(subprocess.run(["python3", str(helper), *command], check=False).returncode, 0)
            result = self._run_download(root, self._archive(root, sidecar))
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["download"], "passed")
            self.assertEqual(status["scan"], "passed", status)
            self.assertEqual(status["triage"]["gateResults"]["overall_exit_code"], 7)
            self.assertEqual(status["triage"]["gateResults"]["gates"]["workload"]["state"], "failed")
            self.assertEqual(status["gateAcceptance"], "failed")
            self.assertEqual(status["error"], "gate-results-acceptance-failed")

    def test_phase_timing_failure_preserves_safe_triage_and_original_gate(self):
        """Timing evidence failure cannot discard a valid archive or workload result."""

        for failure, expected_error in (
            ("download", "phase-timing-download-failed"),
            ("invalid", "phase-timing-invalid"),
            ("oversize", "phase-timing-too-large"),
        ):
            for workload_exit in (0, 7):
                with self.subTest(failure=failure, workload_exit=workload_exit), tempfile.TemporaryDirectory(
                    prefix="heph-gcp-phase-timing-failure-"
                ) as directory:
                    root = Path(directory)
                    gate_results = self._cooking_gate_results(root, workload_exit=workload_exit)
                    archive = self._archive(root, gate_results)
                    result = self._run_download(
                        root,
                        archive,
                        expected_mode="gcp-cooking",
                        phase_timing_failure=failure,
                    )
                    self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                    status = json.loads((root / "status.json").read_text(encoding="utf-8"))
                    self.assertEqual(status["download"], "passed")
                    self.assertEqual(status["scan"], "passed")
                    self.assertEqual(status["phaseTiming"], "unavailable")
                    self.assertEqual(status["error"], expected_error)
                    self.assertEqual(status["timingAcceptance"], "failed")
                    self.assertEqual(status["gateAcceptance"], "passed" if workload_exit == 0 else "failed")
                    if workload_exit:
                        self.assertEqual(status["gateAcceptanceError"], "gate-results-acceptance-failed")
                    self.assertEqual(status["triage"]["gateResults"]["overall_exit_code"], workload_exit)
                    self.assertEqual(status["archiveBytes"], archive.stat().st_size)
                    self.assertEqual(status["archiveSha256"], hashlib.sha256(archive.read_bytes()).hexdigest())
                    self.assertFalse((root / "gcp-cooking-phase-timing.json").exists())
                    self.assertFalse((root / "gcp-cooking-phase-timing.json.download").exists())

    def test_missing_current_gate_sidecar_preserves_safe_triage(self):
        """A current archive missing its sidecar fails after safe triage."""

        with tempfile.TemporaryDirectory(prefix="heph-gcp-missing-gate-") as directory:
            root = Path(directory)
            runtime_hash = hashlib.sha256((ROOT / "gcp-cooking-run.sh").read_bytes()).hexdigest()
            (root / "gcp-diagnostics-gate-expectation").write_text(
                f"gcp-cooking {runtime_hash}\n", encoding="utf-8"
            )
            result = self._run_download(root, self._archive(root))
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["download"], "passed")
            self.assertEqual(status["scan"], "passed")
            self.assertEqual(status["triage"]["collectionStatus"], "complete")
            self.assertEqual(status["gateValidation"], "failed")
            self.assertEqual(status["gateAcceptance"], "failed")
            self.assertEqual(status["error"], "gate-results-acceptance-failed")

    def test_triage_projects_denial_and_latest_snapshot_correlation(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-") as directory:
            root = Path(directory)
            self._archive(root)
            triage = TRIAGE.summarize(root / "bundle")
            self.assertEqual(
                triage["denial"],
                {
                    "denial_stage": "session-authentication",
                    "denial_class": "authentication_denied",
                    "run_id": "00000000-0000-4000-8000-000000000004",
                },
            )
            self.assertEqual(len(triage["attempts"]), 1)
            self.assertEqual(
                triage["attempts"][0]["attempt_run_id"],
                triage["denial"]["run_id"],
            )
            runtime = root / "bundle" / "sources" / "runtime-structured"
            runtime.write_text(
                runtime.read_text(encoding="utf-8")
                + "HEPHAESTUS_RUNTIME denial_stage=decryption denial_class=other_failure "
                "run_id=00000000-0000-4000-8000-000000000099\n",
                encoding="utf-8",
            )
            self.assertEqual(
                TRIAGE.summarize(root / "bundle")["denial"]["run_id"],
                "00000000-0000-4000-8000-000000000004",
            )
            self.assertEqual(triage["sources"]["truncatedCount"], 1)
            self.assertEqual(triage["sources"]["unavailableCount"], 1)
            # Older retained summaries have no projector state; keep their
            # typed failure visible while making the missing report state
            # explicit for callers.
            self.assertEqual(triage["browser"]["status"], "failed")
            self.assertEqual(triage["browser"]["report_state"], "unknown")
            self.assertEqual(triage["browser"]["failure_metadata"], [])

    def test_triage_projects_only_typed_failure_fields(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-failure-") as directory:
            root = Path(directory)
            self._archive(root)
            runtime = root / "bundle" / "sources" / "runtime-structured"
            runtime.write_text(
                "HEPH_GCP_TEST test=rust-panic location=examples/cooking/tests/scenario.rs:1697:28\n"
                "HEPH_GCP_RUNTIME error_class=guest-start-failed status=failed "
                "run_id=00000000-0000-4000-8000-000000000004\n"
                "HEPH_GCP_KVM_BUILD_ERROR phase=real-libkrun-smoke status=1 log=/tmp/private.log\n"
                "HEPH_GCP_RUNNER_IMAGE_READINESS tool=rust class=rust-not-runnable phase=runner-image-runtime\n"
                '{"status":"failed","phase":"structured-smoke","exit_code":1,"exit_signal":null,"run_id":7}\n'
                '{"phase":"successful-smoke","exit_code":0}\n'
                '{"test_result":"failed","phase":"evidence","error":"UNKNOWN_PAYLOAD"}\n',
                encoding="utf-8",
            )
            triage = TRIAGE.summarize(root / "bundle")
            self.assertNotIn("UNKNOWN_PAYLOAD", json.dumps(triage["failures"]))
            self.assertNotIn("error", triage["failures"][-1])
            self.assertIn(
                {"source": "runtime-structured", "exit_code": 1, "phase": "real-libkrun-smoke", "correlated": False},
                triage["failures"],
            )
            self.assertIn(
                {"source": "runtime-structured", "error_class": "rust-not-runnable", "phase": "runner-image-runtime", "test": "rust", "correlated": False},
                triage["failures"],
            )
            self.assertIn(
                {"source": "runtime-structured", "exit_code": 1, "phase": "structured-smoke", "status": "failed", "correlated": False},
                triage["failures"],
            )
            self.assertNotIn("successful-smoke", json.dumps(triage["failures"]))

    def test_triage_retains_safe_technical_context_without_promoting_caught_panics(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-context-") as directory:
            root = Path(directory)
            self._archive(root)
            runtime = root / "bundle" / "sources" / "runtime-structured"
            runtime.write_text(
                "HEPH_GCP_TEST test=rust-panic location=crates/hephaestus-app/tests/../../../examples/cooking/tests/confinement.rs:431:49\n"
                "HEPH_GCP_RUNTIME error=not-found errno=ENOENT\n"
                "HEPH_GCP_COOKING event=workload-result operation=cooking-workload phase=cooking status=failed exit_code=1\n"
                "HEPHAESTUS_GCP_COOKING: FAIL phase=gcp-cooking exit=1 revision=" + "a" * 40 + "\n"
                "HEPH_GCP_RUNTIME error=private-payload errno=ENOENT location=/tmp/private.log\n",
                encoding="utf-8",
            )
            triage = TRIAGE.summarize(root / "bundle")
            context = triage["technicalContext"]
            self.assertIn(
                {
                    "source": "runtime-structured",
                    "order": 1,
                    "kind": "rust-panic",
                    "test": "rust-panic",
                    "source_file": "examples/cooking/tests/confinement.rs",
                    "source_line": 431,
                    "source_column": 49,
                    "correlated": False,
                },
                context,
            )
            self.assertIn(
                {
                    "source": "runtime-structured",
                    "order": 2,
                    "kind": "runtime-error",
                    "error_class": "not-found",
                    "errno": "ENOENT",
                    "correlated": False,
                },
                context,
            )
            self.assertIn(
                {
                    "source": "runtime-structured",
                    "order": 3,
                    "kind": "cooking-result",
                    "event": "workload-result",
                    "operation": "cooking-workload",
                    "phase": "cooking",
                    "status": "failed",
                    "exit_code": 1,
                    "correlated": False,
                },
                context,
            )
            self.assertIn(
                {
                    "source": "runtime-structured",
                    "order": 4,
                    "kind": "cooking-terminal",
                    "phase": "gcp-cooking",
                    "status": "failed",
                    "exit_code": 1,
                    "correlated": False,
                },
                context,
            )
            encoded = json.dumps(context)
            self.assertNotIn("private-payload", encoded)
            self.assertNotIn("private.log", encoded)
            self.assertEqual(
                [failure for failure in triage["failures"] if failure.get("test") == "rust-panic"],
                [],
            )

    def test_triage_context_prioritizes_late_failures_and_deduplicates_sources(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-context-cap-") as directory:
            root = Path(directory)
            self._archive(root)
            runtime = root / "bundle" / "sources" / "runtime-structured"
            runtime.write_text(
                "\n".join(
                    [
                        *(
                            f"HEPH_GCP_TEST test=examples::cooking::tests::passed_{index} status=passed"
                            for index in range(60)
                        ),
                        "HEPH_GCP_RUNTIME error=not-found errno=ENOENT",
                        "HEPH_GCP_RUNTIME error=not-found errno=ENOENT",
                        "HEPH_GCP_COOKING event=workload-result operation=cooking-workload phase=cooking status=failed exit_code=7",
                        "HEPH_GCP_RUNTIME error=private-payload errno=not-a-real-errno location=/tmp/private.log",
                        "HEPH_GCP_COOKING event=workload-result operation=unknown phase=unsafe status=failed exit_code=999",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            context = TRIAGE._project_technical_context(
                root / "bundle",
                [
                    {"label": "runtime-structured", "path": "sources/runtime-structured"},
                    {"label": "runtime-structured", "path": "sources/runtime-structured"},
                ],
                [],
            )
            self.assertLessEqual(len(context), TRIAGE.TECHNICAL_CONTEXT_LIMIT)
            self.assertTrue(any(item.get("error_class") == "not-found" for item in context))
            self.assertTrue(any(item.get("exit_code") == 7 for item in context))
            self.assertEqual(
                sum(item.get("error_class") == "not-found" for item in context),
                1,
            )
            self.assertFalse(any(item.get("source_file") == "tmp/private.log" for item in context))
            self.assertFalse(any(item.get("phase") == "unsafe" for item in context))
            self.assertNotIn("private-payload", json.dumps(context))

    def test_triage_context_keeps_late_priority_tail_and_cross_source_unique_content(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-context-priority-tail-") as directory:
            root = Path(directory)
            self._archive(root)
            runtime = root / "bundle" / "sources" / "runtime-structured"
            runtime.write_text(
                "\n".join(
                    [
                        *(
                            f"HEPH_GCP_TEST test=rust-panic location=examples/cooking/tests/confinement.rs:{index}:1"
                            for index in range(1, 61)
                        ),
                        "HEPH_GCP_RUNTIME error=not-found errno=ENOENT",
                        "HEPH_GCP_COOKING event=workload-result operation=cooking-workload phase=cooking status=failed exit_code=7",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            duplicate = root / "bundle" / "sources" / "runtime-log"
            duplicate.write_text(runtime.read_text(encoding="utf-8"), encoding="utf-8")
            context = TRIAGE._project_technical_context(
                root / "bundle",
                [
                    {"label": "runtime-structured", "path": "sources/runtime-structured"},
                    {"label": "runtime-log", "path": "sources/runtime-log"},
                ],
                [],
            )
            self.assertEqual(len(context), TRIAGE.TECHNICAL_CONTEXT_LIMIT)
            self.assertTrue(any(item.get("error_class") == "not-found" for item in context))
            self.assertTrue(any(item.get("exit_code") == 7 for item in context))
            self.assertEqual(
                sum(item.get("source_line") == index for item in context for index in range(1, 61)),
                48,
            )
            self.assertEqual(
                sum(item.get("kind") == "runtime-error" for item in context),
                1,
            )
            self.assertEqual(
                sum(item.get("kind") == "cooking-result" for item in context),
                1,
            )
            self.assertEqual(
                sum(item.get("kind") == "rust-panic" and item.get("source") == "runtime-structured" for item in context),
                48,
            )

    def test_triage_projects_collector_failure_aliases_from_all_evidence_sources(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-aliases-") as directory:
            root = Path(directory)
            self._archive(root)
            (root / "bundle" / "sources" / "test-output").write_text(
                "HEPHAESTUS_GCP_COOKING FAIL phase=evidence exit=1 "
                "reason_class=browser-tests-not-passed error=private-error-payload\n",
                encoding="utf-8",
            )
            (root / "bundle" / "sources" / "browser-summary").write_text(
                '{"status":"failed","phase":"browser","test":"checkout",'
                '"component":"browser-e2e","result_origin":"playwright-report",'
                '"exit_code":1,"error":"private-request-body"}\n',
                encoding="utf-8",
            )
            (root / "bundle" / "sources" / "runtime-structured").write_text(
                "HEPH_GCP_DIAGNOSTICS test_result=1 phase=evidence\n"
                "HEPH_GCP_DIAGNOSTICS test_result=0 phase=successful-probe\n",
                encoding="utf-8",
            )
            failures = TRIAGE.summarize(root / "bundle")["failures"]
            self.assertIn(
                {
                    "source": "test-output",
                    "phase": "evidence",
                    "status": "failed",
                    "exit_code": 1,
                    "reason_class": "browser-tests-not-passed",
                    "correlated": False,
                },
                failures,
            )
            self.assertIn(
                {
                    "source": "browser-summary",
                    "phase": "browser",
                    "test": "checkout",
                    "status": "failed",
                    "component": "browser-e2e",
                    "result_origin": "playwright-report",
                    "exit_code": 1,
                    "correlated": False,
                },
                failures,
            )
            self.assertIn(
                {
                    "source": "runtime-structured",
                    "phase": "evidence",
                    "exit_code": 1,
                    "correlated": False,
                },
                failures,
            )
            serialized = json.dumps(failures)
            self.assertNotIn("private-error-payload", serialized)
            self.assertNotIn("private-request-body", serialized)
            self.assertNotIn("successful-probe", serialized)

    def test_triage_separates_workload_and_browser_result_origins(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-origins-") as directory:
            root = Path(directory)
            self._archive(root)
            (root / "bundle" / "sources" / "runtime-structured").write_text(
                "HEPH_GCP_COOKING event=workload-result operation=cooking-workload "
                "phase=cooking status=failed exit_code=7\n"
                "HEPH_GCP_COOKING event=evidence-scan operation=evidence-scan "
                "phase=evidence status=passed exit_code=0\n"
                "HEPH_GCP_COOKING event=browser-report-validation operation=browser-report-validation "
                "phase=evidence status=failed report_state=partial reason=incomplete-phases exit_code=3\n",
                encoding="utf-8",
            )
            (root / "bundle" / "sources" / "browser-summary").write_text(
                '{"status":"not-run","suite":"cooking-playwright",'
                '"test":"browser-journey","phase":"browser",'
                '"component":"browser-e2e","result_origin":"no-browser-report"}\n',
                encoding="utf-8",
            )
            failures = TRIAGE.summarize(root / "bundle")["failures"]
            self.assertIn(
                {
                    "source": "runtime-structured",
                    "event": "workload-result",
                    "operation": "cooking-workload",
                    "phase": "cooking",
                    "status": "failed",
                    "exit_code": 7,
                    "correlated": False,
                },
                failures,
            )
            self.assertNotIn("browser-journey", json.dumps(failures))
            self.assertEqual(
                TRIAGE.summarize(root / "bundle")["runtimeResults"],
                [
                    {
                        "source": "runtime-structured",
                        "event": "workload-result",
                        "operation": "cooking-workload",
                        "phase": "cooking",
                        "status": "failed",
                        "exit_code": 7,
                    },
                    {
                        "source": "runtime-structured",
                        "event": "evidence-scan",
                        "operation": "evidence-scan",
                        "phase": "evidence",
                        "status": "passed",
                        "exit_code": 0,
                    },
                    {
                        "source": "runtime-structured",
                        "event": "browser-report-validation",
                        "operation": "browser-report-validation",
                        "phase": "evidence",
                        "status": "failed",
                        "exit_code": 3,
                        "report_state": "partial",
                        "reason": "incomplete-phases",
                    },
                ],
            )
            browser = TRIAGE.summarize(root / "bundle")["browser"]
            self.assertEqual(browser["status"], "not-run")
            self.assertEqual(browser["report_state"], "unknown")
            self.assertIsNone(browser["counts"])
            self.assertEqual(browser["failure_metadata"], [])

    def test_triage_preserves_projector_browser_schema_without_raw_error(self):
        """Exercise the real Playwright projector through collector and triage."""

        with tempfile.TemporaryDirectory(prefix="heph-gcp-browser-projector-") as directory:
            root = Path(directory)
            evidence = root / "playwright"
            report_dir = evidence / "browser.post-operation"
            report_dir.mkdir(parents=True)
            report = {
                "config": {"rootDir": "/repo/e2e/playwright"},
                "suites": [{
                    "file": "cooking-tests/cooking-post-operation.spec.ts",
                    "title": "cooking-post-operation.spec.ts",
                    "specs": [{
                        "file": "cooking-tests/cooking-post-operation.spec.ts",
                        "title": "cooking post-operation controls, provenance, recovery, and denial",
                        "tests": [{"results": [{
                            "status": "failed",
                            "error": {
                                "message": "expect(locator).toContainText()\nsecret=private",
                                "stack": "Error: secret=private",
                            },
                            "errorLocation": {
                                "file": "/repo/e2e/playwright/cooking-tests/cooking-post-operation.spec.ts",
                                "line": 106,
                                "column": 7,
                            },
                        }]}],
                    }],
                }],
            }
            (report_dir / "playwright-report.json").write_text(json.dumps(report), encoding="utf-8")
            projected = PROJECTOR.project(evidence)
            self.assertEqual(projected["report_state"], "complete")
            self.assertEqual(projected["counts"], {"passed": 0, "failed": 1, "skipped": 0, "timed_out": 0})
            summary_path = root / "browser-summary.json"
            summary_path.write_text(json.dumps(projected), encoding="utf-8")
            bundle = root / "bundle"
            self.assertEqual(
                COLLECTOR.main([
                    "--output-dir", str(bundle),
                    "--source", f"browser-summary={summary_path}",
                ]),
                0,
            )
            browser = TRIAGE.summarize(bundle)["browser"]
            self.assertEqual(browser["status"], "failed")
            self.assertEqual(browser["report_state"], "complete")
            self.assertEqual(browser["counts"], {"passed": 0, "failed": 1, "skipped": 0, "timed_out": 0})
            self.assertEqual(browser["observed_phases"], ["post-operation"])
            self.assertEqual(browser["passed_phases"], [])
            self.assertEqual(browser["failure_metadata"], [{
                "test_id": "cooking-post-operation",
                "phase": "post-operation",
                "status": "failed",
                "error_class": "assertion",
                "matcher": "toContainText",
                "source_file": "e2e/playwright/cooking-tests/cooking-post-operation.spec.ts",
                "source_line": 106,
                "source_column": 7,
                "source_location_kind": "error",
            }])
            self.assertNotIn("private", json.dumps(browser).lower())

            # The compatibility reader accepts the producer's complete
            # two-phase enum but rejects duplicate or unknown phase values.
            projected["passed_phases"] = ["initial", "post-operation"]
            summary_path.write_text(json.dumps(projected), encoding="utf-8")
            valid_bundle = root / "valid-bundle"
            self.assertEqual(
                COLLECTOR.main([
                    "--output-dir", str(valid_bundle),
                    "--source", f"browser-summary={summary_path}",
                ]),
                0,
            )
            self.assertEqual(
                TRIAGE.summarize(valid_bundle)["browser"]["passed_phases"],
                ["initial", "post-operation"],
            )
            projected["passed_phases"] = ["initial", "initial"]
            summary_path.write_text(json.dumps(projected), encoding="utf-8")
            invalid_bundle = root / "invalid-bundle"
            self.assertEqual(
                COLLECTOR.main([
                    "--output-dir", str(invalid_bundle),
                    "--source", f"browser-summary={summary_path}",
                ]),
                1,
            )

    def test_triage_projects_bounded_browser_observations_from_collector(self):
        """Collector-normalized browser lines become typed, unassociated facts."""

        with tempfile.TemporaryDirectory(prefix="heph-gcp-browser-observations-") as directory:
            root = Path(directory)
            raw = root / "raw"
            raw.mkdir()
            (raw / "runtime.log").write_text(
                "ERROR: expect(locator).toBeVisible() failed\n"
                "    at /srv/hephaestus/e2e/playwright/cooking-tests/cooking-live-review.spec.ts:203:11\n"
                "ERROR: expect(received).toBe(expected) body=PRIVATE_BODY\n"
                "ERROR: /tmp/private.spec.ts:1:2\n",
                encoding="utf-8",
            )
            (raw / "test-output.log").write_text(
                "AssertionError: expect(locator).toHaveText(expected)\n"
                "ERROR: Timed out 5000ms waiting for expect(locator).toBeVisible()\n"
                "ERROR: expect(received).toBe(expected) request body=PRIVATE_BODY\n"
                "    at /tmp/private.spec.ts:9:4\n",
                encoding="utf-8",
            )
            bundle = root / "bundle"
            self.assertEqual(
                COLLECTOR.main([
                    "--output-dir", str(bundle),
                    "--source", f"runtime-log={raw / 'runtime.log'}",
                    "--source", f"test-output={raw / 'test-output.log'}",
                ]),
                0,
            )
            runtime_projected = (bundle / "sources" / "runtime-log").read_text(encoding="utf-8")
            test_projected = (bundle / "sources" / "test-output").read_text(encoding="utf-8")
            self.assertIn("error=expect(locator).toBeVisible() failed", runtime_projected)
            self.assertIn(
                "location=/srv/hephaestus/e2e/playwright/cooking-tests/cooking-live-review.spec.ts:203:11",
                runtime_projected,
            )
            self.assertIn("assertion=expect(locator).toHaveText(expected)", test_projected)
            self.assertIn("error=Timed out 5000ms waiting for expect(locator).toBeVisible()", test_projected)
            self.assertNotIn("PRIVATE_BODY", runtime_projected + test_projected)

            observations = TRIAGE.summarize(bundle)["browserObservations"]
            self.assertEqual(
                observations,
                [
                    {
                        "source": "runtime-log", "order": 1, "kind": "assertion",
                        "error_class": "assertion-failure", "matcher": "toBeVisible",
                    },
                    {
                        "source": "runtime-log", "order": 2, "kind": "location",
                        "file": "cooking-live-review.spec.ts", "line": 203, "column": 11,
                    },
                    {
                        "source": "test-output", "order": 1, "kind": "assertion",
                        "error_class": "assertion-failure", "matcher": "toHaveText",
                    },
                    {
                        "source": "test-output", "order": 2, "kind": "error",
                        "error_class": "timeout", "matcher": "toBeVisible",
                    },
                ],
            )
            # A location is informational and remains separate from an error
            # in another source; no synthetic failure association is emitted.
            self.assertTrue(all("failure" not in observation for observation in observations))

    def test_triage_caps_browser_observations_and_rejects_malicious_normalized_text(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-browser-observations-cap-") as directory:
            root = Path(directory)
            self._archive(root)
            output = root / "bundle" / "sources" / "test-output"
            output.write_text(
                "\n".join(
                    ["assertion=expect(received).toBe(expected)"] * 70
                    + [
                        "error=expect(received).toBe(expected) JSON_BODY=PRIVATE",
                        "assertion=expected PRIVATE_BODY",
                        "location=/tmp/private.spec.ts:1:2",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            observations = TRIAGE.summarize(root / "bundle")["browserObservations"]
            self.assertEqual(len(observations), TRIAGE.BROWSER_OBSERVATION_LIMIT)
            self.assertEqual(observations[0]["order"], 1)
            self.assertEqual(observations[-1]["order"], 64)
            serialized = json.dumps(observations)
            self.assertNotIn("PRIVATE", serialized)
            self.assertNotIn("private.spec.ts", serialized)

    def test_triage_does_not_count_passed_tests_or_caught_panics_as_failures(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-test-count-") as directory:
            root = Path(directory)
            self._archive(root)
            passed = "\n".join(
                f"HEPH_GCP_TEST test=examples::cooking::tests::passed_{index} status=passed"
                for index in range(60)
            )
            caught_panics = "\n".join(
                f"HEPH_GCP_TEST test=rust-panic-{index} location=examples/cooking/tests/scenario.rs:{index + 10}:1"
                for index in range(60)
            )
            (root / "bundle" / "sources" / "test-output").write_text(
                passed
                + "\n"
                + caught_panics
                + "\nHEPH_GCP_TEST test=examples::cooking::tests::actual_failure status=failed\n",
                encoding="utf-8",
            )
            (root / "bundle" / "sources" / "browser-summary").write_text(
                '{"status":"not-run","phase":"browser","test":"browser-journey",'
                '"component":"browser-e2e","result_origin":"no-browser-report"}\n',
                encoding="utf-8",
            )
            failures = TRIAGE.summarize(root / "bundle")["failures"]
            self.assertEqual(
                failures,
                [{
                    "source": "test-output",
                    "test": "examples::cooking::tests::actual_failure",
                    "status": "failed",
                    "correlated": False,
                }],
            )

    def test_triage_preserves_terminal_retry_marker_when_snapshot_is_stale(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-retry-") as directory:
            root = Path(directory)
            self._archive(root)
            runtime = root / "bundle" / "sources" / "runtime-structured"
            runtime.write_text(
                runtime.read_text(encoding="utf-8")
                + "HEPH_COOKING_RETRY event=terminal classification=retry-terminal-failed "
                + "lookup_status=ok event_id=00000000-0000-4000-8000-000000000010 "
                + "attempt_id=00000000-0000-4000-8000-000000000011 attempt_number=2 "
                + "run_id=00000000-0000-4000-8000-000000000012 attempt_state=failed "
                + "run_state=failed run_outcome=failed exit_code=7 exit_signal=none\n",
                encoding="utf-8",
            )
            retry = TRIAGE.summarize(root / "bundle")["retry"]
            self.assertEqual(retry["classification"], "retry-terminal-failed")
            self.assertEqual(retry["lookup_status"], "ok")
            self.assertEqual(retry["attempt_number"], 2)
            self.assertEqual(retry["exit_code"], 7)
            self.assertIsNone(retry["exit_signal"])
            self.assertEqual(retry["run_state"], "failed")
            self.assertEqual(retry["run_outcome"], "failed")
            self.assertEqual(
                retry["correlated"],
                {"event_id": False, "attempt_id": False, "run_id": False, "same_snapshot_row": False},
            )

    def test_triage_matches_retry_ids_only_when_they_share_snapshot_row(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-retry-correlation-") as directory:
            root = Path(directory)
            self._archive(root)
            event_id = "00000000-0000-4000-8000-000000000010"
            attempt_id = "00000000-0000-4000-8000-000000000011"
            run_id = "00000000-0000-4000-8000-000000000012"
            (root / "bundle" / "lineage.jsonl").write_text(
                json.dumps(
                    {
                        "event_id": event_id,
                        "attempt_id": attempt_id,
                        "attempt_run_id": run_id,
                        "attempt_number": 2,
                        "attempt_state": "failed",
                        "run_state": "failed",
                        "run_outcome": "failed",
                        "exit_code": 7,
                        "exit_signal": None,
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            runtime = root / "bundle" / "sources" / "runtime-structured"
            runtime.write_text(
                runtime.read_text(encoding="utf-8")
                + "HEPH_COOKING_RETRY event=terminal classification=retry-terminal-failed "
                + f"lookup_status=ok event_id={event_id} attempt_id={attempt_id} "
                + f"attempt_number=2 run_id={run_id} attempt_state=failed run_state=failed "
                + "run_outcome=failed exit_code=7 exit_signal=none\n",
                encoding="utf-8",
            )
            triage = TRIAGE.summarize(root / "bundle")
            self.assertEqual(triage["attempts"][0]["exit_code"], 7)
            self.assertIsNone(triage["attempts"][0]["exit_signal"])
            self.assertEqual(
                triage["retry"]["correlated"],
                {"event_id": True, "attempt_id": True, "run_id": True, "same_snapshot_row": True},
            )

    def test_triage_orders_rust_offset_datetime_timestamps(self):
        attempt_id = "00000000-0000-4000-8000-000000000099"
        rows = [
            {
                "attempt_id": attempt_id,
                "attempt_state": "running",
                "run_updated_at": "2026-09-11 18:25:06.123456789 +00:00:00",
            },
            {
                "attempt_id": attempt_id,
                "attempt_state": "failed",
                "run_updated_at": "2026-09-11 18:25:06.123456790 +00:00:00",
            },
        ]
        latest = TRIAGE._latest_attempts(rows)
        self.assertEqual(latest[0]["attempt_state"], "failed")

    def test_triage_normalizes_rust_timestamp_variants_and_rejects_invalid(self):
        self.assertEqual(
            TRIAGE._timestamp_sort_key("2026-09-11 18:25:06.38 +00:00:00"),
            TRIAGE._timestamp_sort_key("2026-09-11T19:25:06.38 +01:00:00"),
        )
        self.assertEqual(
            TRIAGE._timestamp_sort_key("2026-09-11 4:25:06.38 +00:00:00"),
            TRIAGE._timestamp_sort_key("2026-09-11 04:25:06.38 +00:00:00"),
        )
        with self.assertRaises(ValueError):
            TRIAGE._timestamp_sort_key("2026-09-11 18:25:06.38")
        with self.assertRaises(ValueError):
            TRIAGE._timestamp_sort_key("2026-09-11 18:25:06.38 not-a-zone")

    def test_triage_caps_latest_attempts_and_rejects_unknown_fields(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-cap-") as directory:
            root = Path(directory)
            self._archive(root)
            lineage = root / "bundle" / "lineage.jsonl"
            rows = []
            for number in range(60):
                rows.append(
                    json.dumps(
                        {
                            "attempt_id": f"00000000-0000-4000-8000-{number:012d}",
                            "attempt_run_id": f"00000000-0000-4000-8000-{number + 100:012d}",
                            "attempt_number": number + 1,
                            "attempt_state": "failed",
                            "run_state": "failed",
                            "run_outcome": "failed",
                            "disposition": "retryable",
                        }
                    )
                )
            lineage.write_text("\n".join(rows) + "\n", encoding="utf-8")
            triage = TRIAGE.summarize(root / "bundle")
            self.assertEqual(len(triage["attempts"]), 50)
            self.assertLessEqual(
                len(json.dumps(triage, separators=(",", ":")).encode()), TRIAGE.MAX_TRIAGE_BYTES
            )
            duplicate_id = "00000000-0000-4000-8000-000000000201"
            latest = TRIAGE._latest_attempts([
                {"attempt_id": duplicate_id, "attempt_number": 1, "attempt_state": "running", "attempt_completed_at": None},
                {"attempt_id": duplicate_id, "attempt_number": 1, "attempt_state": "failed", "attempt_completed_at": "2026-09-11 10:00:00 +00:00:00"},
            ])
            self.assertEqual(latest[0]["attempt_state"], "failed")
            # The source was already projected by the collector in the normal
            # path; this direct mutation models a malformed private archive.
            lineage.write_text(
                json.dumps({"attempt_id": "00000000-0000-4000-8000-000000000099", "unknown": "value"}) + "\n",
                encoding="utf-8",
            )
            with self.assertRaises(ValueError):
                TRIAGE.summarize(root / "bundle")
            lineage.write_text(
                json.dumps(
                    {
                        "attempt_id": "00000000-0000-4000-8000-000000000099",
                        "event_id": "secret",
                    }
                ) + "\n",
                encoding="utf-8",
            )
            with self.assertRaises(ValueError):
                TRIAGE.summarize(root / "bundle")

    def test_triage_output_has_no_secret_values_or_unknown_fields(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-safe-") as directory:
            root = Path(directory)
            self._archive(root)
            result = TRIAGE.summarize(root / "bundle")
            encoded = json.dumps(result, sort_keys=True)
            self.assertNotIn("secret", encoded.lower())
            self.assertNotIn("token", encoded.lower())
            self.assertEqual(set(result), TRIAGE.TRIAGE_FIELDS)
            self.assertEqual(result["collectionStatus"], "complete")
            self.assertEqual(result["rejectedSources"], [])

    def test_triage_rejects_untyped_denial_classification_or_id(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-triage-denial-") as directory:
            root = Path(directory)
            self._archive(root)
            runtime = root / "bundle" / "sources" / "runtime-structured"
            runtime.write_text(
                "HEPHAESTUS_RUNTIME denial_stage=session-authentication "
                "denial_class=not-a-service-class "
                "run_id=00000000-0000-4000-8000-000000000004\n",
                encoding="utf-8",
            )
            with self.assertRaises(ValueError):
                TRIAGE.summarize(root / "bundle")
            runtime.write_text(
                "HEPHAESTUS_RUNTIME denial_stage=session-authentication "
                "denial_class=authentication_denied run_id=not-a-uuid\n",
                encoding="utf-8",
            )
            with self.assertRaises(ValueError):
                TRIAGE.summarize(root / "bundle")

    def _run_download(
        self,
        root: Path,
        archive: Path,
        exit_code: int = 0,
        hang: bool = False,
        cleanup: str = "absent",
        zone: str = "europe-west1-b",
        triage_failure: bool = False,
        source_identity: tuple[str, str, str] | None = None,
        source_api_sha: str | None = None,
        require_source: bool = False,
        expected_mode: str | None = None,
        phase_timing_failure: str | None = None,
    ):
        fake_bin = root / "bin"
        fake_bin.mkdir()
        if triage_failure:
            real_python = shutil.which("python3")
            self.assertIsNotNone(real_python)
            fake_bin.joinpath("python3").write_text(
                "#!/usr/bin/env bash\n"
                "case \"$*\" in\n"
                "  *summarize-cooking-diagnostics.py*) exit 19 ;;\n"
                f"  *) exec {real_python} \"$@\" ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            fake_bin.joinpath("python3").chmod(0o700)
        fake_gcloud = fake_bin / "gcloud"
        fake_gcloud.write_text(
            "#!/usr/bin/env bash\n"
            "set -Eeuo pipefail\n"
            "target_run=\"${GCP_DIAGNOSTICS_SOURCE_RUN_ID:-${GITHUB_RUN_ID:-34599999999}}\"\n"
            "target_attempt=\"${GCP_DIAGNOSTICS_SOURCE_ATTEMPT:-${GITHUB_RUN_ATTEMPT:-1}}\"\n"
            "if [[ \"${1:-} ${2:-} ${3:-}\" == \"compute instances list\" ]]; then\n"
            f"  if [[ \"${{GCP_FAKE_CLEANUP:-absent}}\" == \"present\" ]]; then printf '[{{\"name\":\"heph-kvm-smoke-%s-%s\"}}]\\n' \"$target_run\" \"$target_attempt\"; else printf '[]\\n'; fi\n"
            f"  if [[ \"${{GCP_FAKE_CLEANUP:-absent}}\" == \"permission\" ]]; then echo 'PERMISSION_DENIED: instances.list' >&2; exit 1; fi\n"
            "  exit 0\n"
            "fi\n"
            "if [[ \"${1:-} ${2:-} ${3:-}\" == \"compute instances describe\" ]]; then\n"
            f"  if [[ \"${{GCP_FAKE_CLEANUP:-absent}}\" == \"present\" ]]; then echo '{{}}'; exit 0; fi\n"
            f"  if [[ \"${{GCP_FAKE_CLEANUP:-absent}}\" == \"permission\" ]]; then echo 'PERMISSION_DENIED: instances.get' >&2; exit 1; fi\n"
            "  echo \"The resource 'projects/hephaestus-508000/zones/europe-west1-b/instances/heph-kvm-smoke-${target_run}-${target_attempt}' was not found\" >&2\n"
            "  exit 1\n"
            "fi\n"
            f"if [[ \"${{1:-}} ${{2:-}}\" == \"storage cp\" ]]; then\n"
            f"  if [[ \"${{3:-}}\" == *.phase-timing.json && \"${{HEPH_FAKE_PHASE_TIMING_FAILURE:-}}\" == download ]]; then exit 17; fi\n"
            f"  if [[ \"${{3:-}}\" == *.phase-timing.json && \"${{HEPH_FAKE_PHASE_TIMING_FAILURE:-}}\" == invalid ]]; then printf '{{}}\\n' >\"$4\"; exit 0; fi\n"
            f"  if [[ \"${{3:-}}\" == *.phase-timing.json && \"${{HEPH_FAKE_PHASE_TIMING_FAILURE:-}}\" == oversize ]]; then head -c 65537 /dev/zero >\"$4\"; exit 0; fi\n"
            f"  {'sleep 5' if hang else f'if (( {exit_code} == 0 )); then cp {archive} \"$4\"; fi'}\n"
            f"  exit {exit_code}\n"
            "fi\n"
            "exit 2\n",
            encoding="utf-8",
        )
        fake_gcloud.chmod(fake_gcloud.stat().st_mode | stat.S_IXUSR)
        if source_identity is not None:
            source_run_id, source_attempt, source_sha = source_identity
            fake_curl = fake_bin / "curl"
            api_sha = source_sha if source_api_sha is None else source_api_sha
            fake_curl.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "printf '%s\\n' \"$*\" > \"${GCP_FAKE_CURL_ARGS:?}\"\n"
                "printf '{\"head_sha\":\"%s\",\"path\":\".github/workflows/cooking-e2e.yml\",\"head_branch\":\"main\",\"run_attempt\":%s}\n' "
                f"'{api_sha}' '{source_attempt}'\n",
                encoding="utf-8",
            )
            fake_curl.chmod(0o700)
        env = os.environ | {
            "PATH": f"{fake_bin}:{os.environ['PATH']}",
            "GITHUB_RUN_ID": "34599999999",
            "GITHUB_RUN_ATTEMPT": "1",
            "GITHUB_SHA": "a" * 40,
            "GCP_DIAGNOSTICS_ARCHIVE": str(root / "download.tar.gz"),
            "GCP_DIAGNOSTICS_STATUS": str(root / "status.json"),
            "GCP_DIAGNOSTICS_DOWNLOAD_TIMEOUT_SECONDS": "1" if hang else "60",
            "GCP_FAKE_CLEANUP": cleanup,
            "GCP_ZONE": zone,
            "RUNNER_TEMP": str(root),
        }
        if expected_mode is not None:
            env["GCP_DIAGNOSTICS_EXPECT_MODE"] = expected_mode
        if phase_timing_failure is not None:
            env["GCP_EXPECT_PHASE_TIMING"] = "true"
            env["HEPH_FAKE_PHASE_TIMING_FAILURE"] = phase_timing_failure
        if source_identity is not None:
            source_run_id, source_attempt, source_sha = source_identity
            env.update(
                {
                    "GCP_DIAGNOSTICS_SOURCE_RUN_ID": source_run_id,
                    "GCP_DIAGNOSTICS_SOURCE_ATTEMPT": source_attempt,
                    "GCP_DIAGNOSTICS_SOURCE_SHA": source_sha,
                    "GH_TOKEN": "github-token-fixture",
                    "GITHUB_REPOSITORY": "wimpheling/hephaestus",
                    "GCP_FAKE_CURL_ARGS": str(root / "curl-args.txt"),
                }
            )
        if require_source:
            env["GCP_DIAGNOSTICS_REQUIRE_SOURCE"] = "true"
        return subprocess.run(
            ["bash", str(ROOT / "gcp-kvm-smoke.sh"), "download-diagnostics"],
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_download_rechecks_manifest_and_scan(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostics-") as directory:
            root = Path(directory)
            result = self._run_download(root, self._archive(root))
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["download"], "passed")
            self.assertEqual(status["scan"], "passed")
            self.assertEqual(status["upload"], "verified-by-download")
            self.assertEqual(status["cleanup"], "verified-absent")
            self.assertEqual(status["triage"]["denial"]["denial_class"], "authentication_denied")
            self.assertEqual(len(status["triage"]["attempts"]), 1)
            self.assertEqual(status["triage"]["snapshotStatus"], None)
            self.assertEqual(status["triage"]["sources"]["availableCount"], 6)
            self.assertEqual(status["triage"]["sources"]["missingCount"], 0)

    def test_historical_download_verifies_github_run_identity_without_vm(self):
        source = ("34618088312", "1", "b" * 40)
        with tempfile.TemporaryDirectory(prefix="heph-gcp-historical-diagnostics-") as directory:
            root = Path(directory)
            result = self._run_download(root, self._archive(root), source_identity=source)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(
                status["object"],
                "gs://hephaestus-508000-cooking-diagnostics/cooking/runs/34618088312/1/" + "b" * 40 + ".tar.gz",
            )
            self.assertEqual(status["cleanup"], "verified-absent")
            self.assertEqual(status["scan"], "passed")
            self.assertIn(
                "/actions/runs/34618088312/attempts/1",
                (root / "curl-args.txt").read_text(encoding="utf-8"),
            )

    def test_historical_download_rejects_present_vm_project_wide(self):
        source = ("34618088312", "1", "b" * 40)
        with tempfile.TemporaryDirectory(prefix="heph-gcp-historical-present-") as directory:
            root = Path(directory)
            result = self._run_download(root, self._archive(root), source_identity=source, cleanup="present")
            self.assertNotEqual(result.returncode, 0)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["error"], "cleanup-unverified")
            self.assertFalse((root / "download.tar.gz").exists())

    def test_historical_download_rejects_github_sha_mismatch_before_fetch(self):
        source = ("34618088312", "1", "b" * 40)
        with tempfile.TemporaryDirectory(prefix="heph-gcp-historical-mismatch-") as directory:
            root = Path(directory)
            result = self._run_download(
                root,
                self._archive(root),
                source_identity=source,
                source_api_sha="c" * 40,
            )
            self.assertNotEqual(result.returncode, 0)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["error"], "source-run-identity-mismatch")
            self.assertFalse((root / "download.tar.gz").exists())

    def test_historical_download_requires_all_source_inputs(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-historical-incomplete-") as directory:
            root = Path(directory)
            result = self._run_download(root, self._archive(root), require_source=True)
            self.assertNotEqual(result.returncode, 0)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["error"], "source-identity-incomplete")
            self.assertFalse((root / "download.tar.gz").exists())

    def test_diagnostic_bundle_staging_requires_no_forge_account(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostic-root-") as directory:
            archive = self._archive(Path(directory))
            self.assertTrue(archive.is_file())
            self.assertEqual(pwd.getpwuid(archive.stat().st_uid).pw_uid, os.getuid())

    def test_download_refuses_present_vm_before_fetch(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-present-vm-") as directory:
            root = Path(directory)
            result = self._run_download(root, self._archive(root), cleanup="present")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(json.loads((root / "status.json").read_text())["error"], "cleanup-unverified")
            self.assertNotIn("storage cp", result.stderr)

    def test_download_refuses_inconclusive_cleanup_permission_error(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-cleanup-permission-") as directory:
            root = Path(directory)
            result = self._run_download(root, self._archive(root), cleanup="permission")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(json.loads((root / "status.json").read_text())["error"], "cleanup-unverified")

    def test_download_rejects_zone_outside_reviewed_region(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-wrong-zone-") as directory:
            root = Path(directory)
            result = self._run_download(root, self._archive(root), zone="us-central1-a")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(json.loads((root / "status.json").read_text())["error"], "cleanup-unverified")

    def test_download_failure_is_recorded_and_fails_closed(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostics-") as directory:
            root = Path(directory)
            result = self._run_download(root, self._archive(root), exit_code=23)
            self.assertNotEqual(result.returncode, 0)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["download"], "failed")
            self.assertEqual(status["cleanup"], "verified-absent")
            self.assertEqual(status["upload"], "unknown")
            self.assertEqual(status["scan"], "not-run")

    def test_download_failure_preserves_verified_cleanup_and_scan_failure(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostics-") as directory:
            root = Path(directory)
            malformed = root / "malformed.tar.gz"
            malformed.write_bytes(b"not a gzip archive")
            result = self._run_download(root, malformed)
            self.assertNotEqual(result.returncode, 0)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["cleanup"], "verified-absent")
            self.assertEqual(status["upload"], "verified-by-download")
            self.assertEqual(status["scan"], "failed")

    def test_triage_failure_preserves_download_and_scan_success(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostics-") as directory:
            root = Path(directory)
            archive = self._archive(root)
            result = self._run_download(root, archive, triage_failure=True)
            self.assertNotEqual(result.returncode, 0)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["cleanup"], "verified-absent")
            self.assertEqual(status["upload"], "verified-by-download")
            self.assertEqual(status["download"], "passed")
            self.assertEqual(status["scan"], "passed")
            self.assertEqual(status["triage"], "failed")
            self.assertEqual(status["error"], "triage-projection-failed")
            self.assertEqual(status["archiveBytes"], archive.stat().st_size)
            self.assertEqual(status["archiveSha256"], hashlib.sha256(archive.read_bytes()).hexdigest())

    def test_malformed_archive_is_recorded_and_fails_closed(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostics-") as directory:
            root = Path(directory)
            malformed = root / "malformed.tar.gz"
            malformed.write_bytes(b"not a gzip archive")
            result = self._run_download(root, malformed)
            self.assertNotEqual(result.returncode, 0)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["error"], "archive-format-invalid")

    def test_manifest_source_checksum_mismatch_is_recorded(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostics-") as directory:
            root = Path(directory)
            archive = self._archive(root)
            broken = root / "broken.tar.gz"
            with tarfile.open(archive, "r:gz") as source, tarfile.open(broken, "w:gz") as target:
                for member in source.getmembers():
                    data = source.extractfile(member).read() if member.isfile() else None
                    if member.name.endswith("/sources/serial"):
                        data = b"HEPH_GCP_DIAGNOSTIC tampered\n"
                    if data is None:
                        target.addfile(member)
                    else:
                        member.size = len(data)
                        target.addfile(member, io.BytesIO(data))
            result = self._run_download(root, broken)
            self.assertNotEqual(result.returncode, 0)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["error"], "manifest-validation-failed")

    def test_hanging_provider_download_is_bounded(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostics-") as directory:
            root = Path(directory)
            result = self._run_download(root, self._archive(root), hang=True)
            self.assertNotEqual(result.returncode, 0)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["error"], "provider-download-failed")

    def test_download_credential_scan_failure_is_recorded(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostics-") as directory:
            root = Path(directory)
            archive = self._archive(root)
            broken = root / "credential.tar.gz"
            with tarfile.open(archive, "r:gz") as source:
                members = source.getmembers()
                payloads = {
                    member.name: source.extractfile(member).read()
                    for member in members
                    if member.isfile()
                }
            serial_name = "cooking-diagnostics/sources/serial"
            payloads[serial_name] = COLLECTOR.EVIDENCE.VALUES[0] + b"\n"
            manifest_name = "cooking-diagnostics/manifest.json"
            manifest = json.loads(payloads[manifest_name])
            for record in manifest["sources"]:
                if record["path"] == "sources/serial":
                    record["bytes"] = len(payloads[serial_name])
                    record["sha256"] = hashlib.sha256(payloads[serial_name]).hexdigest()
            payloads[manifest_name] = (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode()
            with tarfile.open(broken, "w:gz") as target:
                for member in members:
                    if not member.isfile():
                        continue
                    data = payloads[member.name]
                    member.size = len(data)
                    target.addfile(member, io.BytesIO(data))
            result = self._run_download(root, broken)
            self.assertNotEqual(result.returncode, 0)
            status = json.loads((root / "status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["error"], "credential-scan-failed")

    def test_full_cooking_producer_paths_and_lineage_status_are_retained(self):
        """Exercise the real producer filenames and status sidecar contract."""
        with tempfile.TemporaryDirectory(prefix="heph-gcp-full-paths-") as directory:
            root = Path(directory)
            evidence = root / "evidence" / "cooking"
            evidence.mkdir(mode=0o700, parents=True)
            serial = evidence / "serial.log"
            serial.write_text("HEPH_GCP_COOKING phase=browser status=failed\n", encoding="utf-8")
            lineage = evidence / "cooking-lineage.jsonl"
            lineage.write_text(
                '{"sampled_at":"2026-09-11 10:00:00 +00:00:00",'
                '"mailbox_id":"00000000-0000-4000-8000-000000000001",'
                '"event_id":"00000000-0000-4000-8000-000000000002",'
                '"attempt_id":"00000000-0000-4000-8000-000000000003",'
                '"attempt_number":1,"attempt_run_id":"00000000-0000-4000-8000-000000000004",'
                '"attempt_state":"failed","attempt_created_at":"2026-09-11 09:59:00 Z",'
                '"attempt_completed_at":"2026-09-11 10:00:00 Z","run_state":"failed",'
                '"run_outcome":"failed","run_created_at":"2026-09-11 09:58:00 Z",'
                '"run_updated_at":"2026-09-11 10:00:00 Z","disposition":"retryable"}\n',
                encoding="utf-8",
            )
            lineage_status = evidence / "cooking-lineage-status.json"
            lineage_status.write_text(
                '{"schema":1,"status":"query_failed","sampled_at":"2026-09-11 10:00:00 Z",'
                '"mailbox_id":"00000000-0000-4000-8000-000000000001",'
                '"event_id":null,"rows":0}\n',
                encoding="utf-8",
            )
            output = root / "bundle"
            archive = root / "bundle.tar.gz"
            self.assertEqual(
                COLLECTOR.main([
                    "--output-dir", str(output),
                    "--source", f"serial={serial}",
                    "--snapshot-jsonl", str(lineage),
                    "--snapshot-status", str(lineage_status),
                    "--archive", str(archive),
                ]),
                0,
            )
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["credentialScan"], "passed")
            self.assertEqual(
                {record["label"] for record in manifest["sources"]},
                {"serial", "lineage", "lineage-status"},
            )
            self.assertEqual(manifest["collectionErrors"], [])
            triage = TRIAGE.summarize(output)
            self.assertEqual(triage["snapshotStatus"]["status"], "query_failed")
            self.assertEqual(triage["snapshotStatus"]["rows"], 0)

    def test_collector_to_triage_accepts_producer_unpadded_hour(self):
        """Keep the real collector/summarizer path compatible with Rust timestamps."""
        with tempfile.TemporaryDirectory(prefix="heph-gcp-unpadded-hour-") as directory:
            root = Path(directory)
            evidence = root / "evidence" / "cooking"
            evidence.mkdir(mode=0o700, parents=True)
            serial = evidence / "serial.log"
            serial.write_text("HEPH_GCP_COOKING phase=running status=passed\n", encoding="utf-8")
            lineage = evidence / "cooking-lineage.jsonl"
            lineage.write_text(
                '{"sampled_at":"2026-09-11 4:00:00 +00:00:00",'
                '"mailbox_id":"00000000-0000-4000-8000-000000000001",'
                '"event_id":"00000000-0000-4000-8000-000000000002",'
                '"attempt_id":"00000000-0000-4000-8000-000000000003",'
                '"attempt_number":1,"attempt_run_id":"00000000-0000-4000-8000-000000000004",'
                '"attempt_state":"completed","attempt_created_at":"2026-09-11 3:59:00 Z",'
                '"attempt_completed_at":"2026-09-11 4:00:00 Z","run_state":"succeeded",'
                '"run_outcome":"succeeded","run_created_at":"2026-09-11 3:58:00 Z",'
                '"run_updated_at":"2026-09-11 4:00:00 Z","disposition":"delivered"}\n',
                encoding="utf-8",
            )
            lineage_status = evidence / "cooking-lineage-status.json"
            lineage_status.write_text(
                '{"schema":1,"status":"ok","sampled_at":"2026-09-11 4:00:00 Z",'
                '"mailbox_id":"00000000-0000-4000-8000-000000000001",'
                '"event_id":null,"rows":1}\n',
                encoding="utf-8",
            )
            output = root / "bundle"
            archive = root / "bundle.tar.gz"
            self.assertEqual(
                COLLECTOR.main([
                    "--output-dir", str(output),
                    "--source", f"serial={serial}",
                    "--snapshot-jsonl", str(lineage),
                    "--snapshot-status", str(lineage_status),
                    "--archive", str(archive),
                ]),
                0,
            )
            self.assertIn(" 4:00:00 ", (output / "lineage.jsonl").read_text(encoding="utf-8"))
            triage = TRIAGE.summarize(output)
            self.assertEqual(len(triage["attempts"]), 1)
            self.assertEqual(triage["attempts"][0]["attempt_state"], "completed")
            self.assertEqual(triage["snapshotStatus"]["status"], "ok")

    def test_full_missing_lineage_is_an_explicit_collector_error(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-missing-lineage-") as directory:
            root = Path(directory)
            source = root / "serial.log"
            source.write_text("HEPH_GCP_COOKING phase=cleanup status=failed\n", encoding="utf-8")
            missing = root / "evidence" / "cooking-lineage.jsonl"
            output = root / "bundle"
            self.assertEqual(
                COLLECTOR.main([
                    "--output-dir", str(output),
                    "--source", f"serial={source}",
                    "--missing-source", f"lineage={missing}",
                ]),
                0,
            )
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            self.assertIn(
                {"label": "lineage", "status": "missing"},
                manifest["collectionErrors"],
            )

    def test_startup_uses_real_paths_and_diagnostic_root_staging(self):
        startup = (ROOT / "gcp-kvm-startup.sh").read_text(encoding="utf-8")
        self.assertIn('snapshot_input="${cooking_evidence_root}/cooking-lineage.jsonl"', startup)
        self.assertIn('snapshot_status_path="${cooking_evidence_root}/cooking-lineage-status.json"', startup)
        self.assertIn("--snapshot-status \"$snapshot_status_path\"", startup)
        self.assertNotIn('install -d -m 0700 -o forge -g forge "$input_root"', startup)
        self.assertIn("diagnostic_probe_completed=true", startup)
        self.assertIn("local expected_fixture=false", startup)
        self.assertIn("TEST-FAIL expected=%s", startup)
        self.assertIn("--config \"$diagnostics_header_file\"", startup)
        self.assertNotIn('-H "Authorization: Bearer $token"', startup)
        self.assertIn(
            'cooking_gate_results_path="${HEPH_GCP_COOKING_GATE_RESULTS_PATH:-/var/log/hephaestus/cooking-gate-results.json}"',
            startup,
        )
        self.assertIn('--source "gate-results=$gate_results_input"', startup)
        self.assertIn('--source "evidence-scan=$evidence_scan_input"', startup)
        self.assertIn('metadata_value cooking-gate-results-helper', startup)
        coordinator = (ROOT / "gcp-kvm-smoke.sh").read_text(encoding="utf-8")
        self.assertIn("GCP_DIAGNOSTICS_EXPECT_GATE_SCRIPT_SHA256", coordinator)
        self.assertIn("finalized gate result helper hash is invalid", coordinator)
        self.assertIn("gate-results-acceptance-failed", coordinator)

    def test_gate_sidecar_finalizer_preserves_sigkill_and_unfinished_states(self):
        startup = ROOT / "gcp-kvm-startup.sh"
        helper = ROOT / "cooking-gate-results.py"
        with tempfile.TemporaryDirectory(prefix="heph-gcp-gate-finalize-") as directory:
            root = Path(directory)
            sidecar = root / "cooking-gate-results.json"
            command = r'''
export HEPH_GCP_COOKING_GATE_RESULTS_PATH="$2"
source "$1"
trap - EXIT
test_mode=diagnostic
revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
diagnostics_gate_helper="$3"
diagnostics_gate_script_sha256="$(sha256sum "$diagnostics_gate_helper" | awk '{print $1}')"
# The startup library normally establishes these anchors before initialization;
# this isolated finalizer test sources the library directly, so provide the
# same bounded clocks instead of allowing a negative timeout argument.
trial_deadline_epoch=$(( $(date +%s) + 30 ))
collection_deadline_epoch=$(( $(date +%s) + 30 ))
initialize_cooking_gate_results
finalize_cooking_gate_results 137
python3 - "$cooking_gate_results_path" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
assert value["finalized"] is True
assert value["supervisor_exit_code"] == 137
assert value["overall_exit_code"] == 137
assert all(gate["state"] == "unknown" and gate["exit_code"] is None
           for gate in value["gates"].values())
PY
python3 - "$cooking_gate_results_path" <<'PY'
import json, sys
path = sys.argv[1]
value = json.load(open(path, encoding="utf-8"))
value["supervisor_exit_code"] = None
json.dump(value, open(path, "w", encoding="utf-8"))
PY
finalize_cooking_gate_results 99
python3 - "$cooking_gate_results_path" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
assert value["overall_exit_code"] == 137
assert value["supervisor_exit_code"] == 99
PY
'''
            env = {
                **os.environ,
                "HEPH_GCP_STARTUP_LIBRARY": "1",
                "HEPH_GCP_WORK_ROOT": str(root / "work"),
            }
            result = subprocess.run(
                [
                    "bash", "-Eeuo", "pipefail", "-c", command, "gate-finalize",
                    str(startup), str(sidecar), str(helper),
                ],
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_diagnostic_gate_writer_runs_real_unsafe_scanner_fixture(self):
        startup = ROOT / "gcp-kvm-startup.sh"
        helper = ROOT / "cooking-gate-results.py"
        scanner = ROOT / "check-browser-evidence.py"
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostic-gates-") as directory:
            root = Path(directory)
            sidecar = root / "cooking-gate-results.json"
            metadata = root / "metadata"
            metadata.mkdir()
            shutil.copy2(scanner, metadata / scanner.name)
            command = r'''
source "$1"
trap - EXIT
test_mode=diagnostic
revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
diagnostics_gate_helper="$3"
diagnostics_gate_script_sha256="$(sha256sum "$diagnostics_gate_helper" | awk '{print $1}')"
collection_deadline_epoch=$(( $(date +%s) + 30 ))
initialize_cooking_gate_results
complete_diagnostic_gate_results
finalize_cooking_gate_results 42
python3 - "$cooking_gate_results_path" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
assert value["finalized"] is True
assert value["overall_exit_code"] == 42
assert value["supervisor_exit_code"] == 42
assert value["gates"]["workload"]["reason_class"] == "workload-failed"
assert value["gates"]["evidence-scan"]["reason_class"] == "evidence-scan-failed"
assert value["gates"]["browser-validation"]["reason_class"] == "browser-validation-failed"
PY
'''
            env = {
                **os.environ,
                "HEPH_GCP_STARTUP_LIBRARY": "1",
                "HEPH_GCP_WORK_ROOT": str(root / "tmp"),
                "HEPH_GCP_DIAGNOSTICS_METADATA_ROOT": str(metadata),
                "HEPH_GCP_COOKING_GATE_RESULTS_PATH": str(sidecar),
                "HEPH_GCP_EVIDENCE_SCAN_STATUS_PATH": str(root / "evidence-scan-status.json"),
            }
            result = subprocess.run(
                ["bash", "-Eeuo", "pipefail", "-c", command, "diagnostic-gates", str(startup), str(sidecar), str(helper), str(root / "tmp"), str(metadata)],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            report = json.loads((root / "evidence-scan-status.json").read_text(encoding="utf-8"))
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["rule"], "browser-secret-org")

    def test_cooking_failure_diagnostics_do_not_retain_systemd_process_arguments(self):
        runner = (ROOT / "gcp-cooking-run.sh").read_text(encoding="utf-8")
        self.assertIn("systemctl show \"$cooking_unit\"", runner)
        self.assertIn("--property=ActiveState,SubState,Result,ExecMainCode,ExecMainStatus,MainPID", runner)
        self.assertNotIn('systemctl status "$cooking_unit"', runner)
        self.assertIn('systemctl kill "$cooking_unit" --kill-who=all --signal=KILL', runner)
        self.assertIn('--property=TimeoutStartSec="${cooking_remaining}s" --property=TimeoutStopSec=15s', runner)
        self.assertNotIn('--property=TimeoutStartSec=15s', runner)
        self.assertIn('timeout --kill-after=1s 5s systemctl show "$cooking_unit"', runner)
        self.assertIn('run_with_deadline timeout --kill-after=2s 15s systemctl stop "$cooking_unit"', runner)

    def test_cooking_result_markers_and_browser_origin_are_component_specific(self):
        runner = (ROOT / "gcp-cooking-run.sh").read_text(encoding="utf-8")
        workload_marker = "event=workload-result operation=cooking-workload"
        evidence_marker = "event=evidence-scan operation=evidence-scan"
        self.assertIn(workload_marker, runner)
        self.assertIn(evidence_marker, runner)
        self.assertIn('scan_status_report="$evidence_root/evidence-scan-status.json"', runner)
        self.assertIn('--status-output "$scan_status_report"', runner)
        self.assertIn("path_sha256", runner)
        self.assertIn("project-playwright-browser-summary.py", runner)
        workload_cleanup_guard = runner.index("if ((status != 0)); then", runner.index(workload_marker))
        self.assertLess(runner.index(workload_marker), workload_cleanup_guard)
        self.assertLess(runner.index(evidence_marker), runner.index('browser-summary.json'))

        with tempfile.TemporaryDirectory(prefix="heph-gcp-browser-summary-") as directory:
            root = Path(directory)
            report_root = root / "browser.1"
            report_root.mkdir()
            report = {
                "config": {"rootDir": "/repo/e2e/playwright"},
                "suites": [{"file": "cooking-tests/cooking-live-review.spec.ts", "title": "suite", "specs": [{
                    "file": "cooking-tests/cooking-live-review.spec.ts",
                    "title": "cooking release install, mailbox, gateway configure, and binding",
                    "tests": [{"results": [{"status": "passed"}]}],
                }]}],
            }
            (report_root / "playwright-report.json").write_text(json.dumps(report), encoding="utf-8")
            passed_summary = PROJECTOR.project(root)
            self.assertEqual(passed_summary["status"], "passed")
            self.assertEqual(passed_summary["counts"]["passed"], 1)
            self.assertEqual(passed_summary["result_origin"], "playwright-report")

            workload_failure = passed_summary.copy()
            workload_failure["workload_exit_code"] = 7
            self.assertEqual(workload_failure["status"], "passed")
            self.assertNotEqual(workload_failure["status"], "failed")

            (report_root / "playwright-report.json").unlink()
            not_run_summary = PROJECTOR.project(root)
            self.assertEqual(not_run_summary["status"], "unknown")
            self.assertEqual(not_run_summary["result_origin"], "playwright-report")
            self.assertEqual(not_run_summary["report_state"], "partial")

            (report_root / "playwright-report.json").write_text("{malformed", encoding="utf-8")
            unknown_summary = PROJECTOR.project(root)
            self.assertEqual(unknown_summary["status"], "unknown")
            self.assertEqual(unknown_summary["result_origin"], "playwright-report")
            self.assertEqual(unknown_summary["report_state"], "malformed")

    def test_coordinator_uses_mode_bound_before_terminal_timeout(self):
        coordinator = (ROOT / "gcp-kvm-smoke.sh").read_text(encoding="utf-8")
        self.assertIn("poll_deadline_epoch=$((trial_start_epoch + 540))", coordinator)
        self.assertIn("poll_deadline_epoch=$((trial_start_epoch + 2400))", coordinator)
        self.assertIn("disposable VM disappeared before a terminal marker", coordinator)
        self.assertNotIn("local deadline=$((SECONDS + 2400))", coordinator)

    def test_diagnostic_create_uses_e2_compatible_maintenance_policy(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-create-argv-") as directory:
            root = Path(directory)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            state = root / "created"
            args_file = root / "create.args"
            fake_bin.joinpath("gcloud").write_text(
                "#!/usr/bin/env bash\nset -Eeuo pipefail\n"
                "if [[ \"$1 $2 $3\" == \"compute regions describe\" ]]; then\n"
                "  printf '{\"quotas\":[{\"metric\":\"INSTANCES\",\"limit\":24,\"usage\":0}]}\\n'; exit 0\n"
                "fi\n"
                "if [[ \"$1 $2 $3\" == \"compute instances create\" ]]; then\n"
                "  printf '%q ' \"$@\" >\"$GCP_FAKE_ARGS\"; touch \"$GCP_FAKE_STATE\"; exit 0\n"
                "fi\n"
                "if [[ \"$1 $2 $3\" == \"compute instances get-serial-port-output\" ]]; then\n"
                "  printf '%s\\n' 'HEPHAESTUS_GCP_DIAGNOSTIC: TEST-FAIL expected=true phase=diagnostic-synthetic exit=42'\n"
                "  printf '%s\\n' 'HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS PASS test_result=expected-failure'\n"
                "  exit 0\n"
                "fi\n"
                "if [[ \"$1 $2 $3\" == \"compute instances describe\" ]]; then\n"
                "  if [[ -f \"$GCP_FAKE_STATE\" ]]; then\n"
                "    printf '{\"labels\":{\"purpose\":\"hephaestus-kvm-smoke\",\"run_id\":\"%s\",\"run_attempt\":\"1\",\"sha\":\"%s\"}}\\n' \"$GITHUB_RUN_ID\" \"$GITHUB_SHA\"; exit 0\n"
                "  fi\n"
                "  printf \"The resource 'instances/heph-kvm-smoke-%s-1' was not found\\n\" \"$GITHUB_RUN_ID\" >&2; exit 1\n"
                "fi\n"
                "if [[ \"$1 $2 $3\" == \"compute instances delete\" ]]; then rm -f \"$GCP_FAKE_STATE\"; exit 0; fi\n"
                "exit 2\n",
                encoding="utf-8",
            )
            fake_bin.joinpath("gcloud").chmod(0o700)
            env = os.environ | {
                "PATH": f"{fake_bin}:{os.environ['PATH']}",
                "GCP_FAKE_STATE": str(state),
                "GCP_FAKE_ARGS": str(args_file),
                "GITHUB_RUN_ID": "34600000001",
                "GITHUB_RUN_ATTEMPT": "1",
                "GITHUB_SHA": "b" * 40,
                "GCP_ZONE": "europe-west1-d",
            }
            result = subprocess.run(
                ["bash", str(ROOT / "gcp-kvm-smoke.sh"), "diagnostic"],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            create_args = args_file.read_text(encoding="utf-8")
            self.assertIn("--machine-type=e2-small", create_args)
            self.assertIn("--maintenance-policy=MIGRATE", create_args)
            self.assertIn("--max-run-duration=10m", create_args)
            self.assertIn("--boot-disk-size=20GB", create_args)
            self.assertNotIn("--maintenance-policy=TERMINATE", create_args)
            self.assertNotIn("--enable-nested-virtualization", create_args)

    def test_real_finish_collects_and_uploads_successful_workload(self):
        result, root = self._run_real_finish("gcp-cooking")
        self.addCleanup(shutil.rmtree, root, True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("HEPHAESTUS_GCP_COOKING: PASS", result.stdout)
        self.assertIn("event=upload status=pass", result.stdout)

    def test_real_finish_preserves_workload_failure_status(self):
        result, root = self._run_real_finish("gcp-cooking", workload="failure")
        self.addCleanup(shutil.rmtree, root, True)
        self.assertEqual(result.returncode, 7, result.stdout + result.stderr)
        self.assertIn("HEPHAESTUS_GCP_COOKING: FAIL", result.stdout)
        self.assertIn("exit=7", result.stdout)
        self.assertIn("event=upload status=pass", result.stdout)
        self.assertNotIn("phase-timing-sidecar status=invalid", result.stdout)
        archive = root / "work" / "tmp" / "cooking-diagnostics.tar.gz"
        self.assertTrue(archive.is_file())
        with tarfile.open(archive, "r:gz") as bundle:
            names = {member.name for member in bundle.getmembers()}
        # Missing workload phases make the timing evidence unavailable.  The
        # raw failure archive still uploads, but no partial projection may be
        # retained as performance evidence.
        self.assertNotIn("cooking-diagnostics/sources/phase-timing", names)

    def test_real_finish_preserves_timeout_status_and_runs_collection(self):
        result, root = self._run_real_finish("gcp-cooking", workload="timeout")
        self.addCleanup(shutil.rmtree, root, True)
        self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
        self.assertIn("event=upload status=pass", result.stdout)
        self.assertIn("HEPHAESTUS_GCP_COOKING: FAIL", result.stdout)
        self.assertIn("exit=124", result.stdout)
        self.assertNotIn("HEPHAESTUS_GCP_COOKING: PASS", result.stdout)
        self.assertFalse((root / "work" / "tmp" / "diagnostics-curl.conf").exists())

    def test_workload_failure_and_upload_failure_preserve_original_status(self):
        result, root = self._run_real_finish("gcp-cooking", upload="403", workload="failure")
        self.addCleanup(shutil.rmtree, root, True)
        self.assertEqual(result.returncode, 7, result.stdout + result.stderr)
        self.assertIn("event=upload status=fail", result.stdout)
        self.assertIn("exit=7", result.stdout)
        self.assertNotIn("HEPHAESTUS_GCP_COOKING: PASS", result.stdout)

    def test_real_finish_diagnostic_expected_failure_is_distinct(self):
        result, root = self._run_real_finish("diagnostic")
        self.addCleanup(shutil.rmtree, root, True)
        self.assertEqual(result.returncode, 42, result.stdout + result.stderr)
        self.assertIn("TEST-FAIL expected=true", result.stdout)
        self.assertIn("DIAGNOSTICS PASS", result.stdout)
        with tarfile.open(root / "work" / "tmp" / "cooking-diagnostics.tar.gz", "r:gz") as archive:
            archive_members = archive.getmembers()
            manifest_member = archive.extractfile("cooking-diagnostics/manifest.json")
            self.assertIsNotNone(manifest_member)
            manifest = json.load(manifest_member)
        self.assertEqual(manifest["collectionStatus"], "partial")
        self.assertEqual(
            manifest["rejectedSources"],
            [{"label": "runtime-log", "reason": "credential-scan-rejected", "status": "rejected"}],
        )
        self.assertFalse(any("credential" in member.name for member in archive_members))
        with tempfile.TemporaryDirectory(prefix="heph-gcp-diagnostic-triage-") as extracted:
            extracted_root = Path(extracted)
            with tarfile.open(root / "work" / "tmp" / "cooking-diagnostics.tar.gz", "r:gz") as bundle:
                bundle.extractall(extracted_root)
            triage = TRIAGE.summarize(extracted_root / "cooking-diagnostics")
        self.assertEqual(triage["collectionStatus"], "partial")
        self.assertEqual(triage["rejectedSources"], manifest["rejectedSources"])

    def test_real_finish_upload_and_scanner_failures_are_explicit_and_clean_headers(self):
        for upload, scanner in (("403", "ok"), ("hang", "ok"), ("ok", "reject")):
            with self.subTest(upload=upload, scanner=scanner):
                result, root = self._run_real_finish("gcp-cooking", upload=upload, scanner=scanner)
                self.addCleanup(shutil.rmtree, root, True)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("HEPHAESTUS_GCP_COOKING: FAIL", result.stdout)
                self.assertNotIn("HEPHAESTUS_GCP_COOKING: PASS", result.stdout)
                self.assertFalse((root / "work" / "tmp" / "diagnostics-curl.conf").exists())
                self.assertFalse((root / "work" / "tmp" / "diagnostics-token.json").exists())

    def test_triage_preserves_typed_evidence_scan_and_legacy_unavailable_state(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-evidence-scan-") as directory:
            root = Path(directory)
            scan = root / "evidence-scan.json"
            scan.write_text(
                '{"schema":1,"status":"failed","rule":"archive-invalid",'
                '"file_class":"archive","path_sha256":"' + "b" * 64 + '",'
                '"checked_files":7,"checked_bytes":1234}\n',
                encoding="utf-8",
            )
            bundle = root / "bundle"
            self.assertEqual(
                COLLECTOR.collect(bundle, [f"evidence-scan={scan}"], None, None, None), 0
            )
            triage = TRIAGE.summarize(bundle)
            self.assertEqual(triage["evidenceScan"]["status"], "failed")
            self.assertEqual(triage["evidenceScan"]["rule"], "archive-invalid")
            self.assertEqual(triage["evidenceScan"]["checked_files"], 7)
            self.assertEqual(triage["evidenceScan"]["path_sha256"], "b" * 64)

            legacy_source = root / "serial.log"
            legacy_source.write_text("HEPH_GCP_KVM_STARTUP event=ready\n", encoding="utf-8")
            legacy_bundle = root / "legacy-bundle"
            self.assertEqual(
                COLLECTOR.collect(legacy_bundle, [f"serial={legacy_source}"], None, None, None), 0
            )
            self.assertEqual(
                TRIAGE.summarize(legacy_bundle)["evidenceScan"],
                {
                    "schema": 1,
                    "status": "unavailable",
                    "rule": "missing-result",
                    "file_class": "none",
                    "path_sha256": None,
                    "checked_files": 0,
                    "checked_bytes": 0,
                },
            )

    def test_triage_rejects_unbounded_evidence_scan_fields(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-evidence-scan-invalid-") as directory:
            root = Path(directory)
            scan = root / "evidence-scan.json"
            scan.write_text(
                '{"schema":1,"status":"passed","rule":"none","file_class":"none",'
                '"path_sha256":null,"checked_files":0,"checked_bytes":0,"context":"raw"}\n',
                encoding="utf-8",
            )
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(root / "bundle", [f"evidence-scan={scan}"], None, None, None)

    def test_triage_derives_scan_result_from_runtime_log_without_sidecar(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-evidence-marker-") as directory:
            root = Path(directory)
            runtime = root / "gcp-cooking-run.log"
            runtime.write_text(
                "HEPH_GCP_COOKING event=workload-result operation=cooking-workload "
                "phase=cooking status=failed exit_code=7\n"
                "HEPH_GCP_COOKING event=evidence-scan operation=evidence-scan phase=evidence "
                "status=failed exit_code=1 report_status=failed rule=browser-secret-org "
                "file_class=content path_sha256=" + "c" * 64 + " checked_files=19 checked_bytes=2048\n"
                "HEPH_GCP_COOKING event=browser-report-validation operation=browser-report-validation "
                "phase=evidence status=passed report_state=complete reason=complete exit_code=0\n",
                encoding="utf-8",
            )
            browser = root / "browser-summary.json"
            browser.write_text(
                '{"status":"passed","phase":"browser","component":"browser-e2e",'
                '"result_origin":"playwright-report"}\n',
                encoding="utf-8",
            )
            bundle = root / "bundle"
            self.assertEqual(
                COLLECTOR.collect(
                    bundle,
                    [f"runtime-log={runtime}", f"browser-summary={browser}"],
                    None,
                    None,
                    None,
                ),
                0,
            )
            triage = TRIAGE.summarize(bundle)
            self.assertEqual(triage["evidenceScan"]["status"], "failed")
            self.assertEqual(triage["evidenceScan"]["rule"], "browser-secret-org")
            self.assertEqual(triage["evidenceScan"]["outer_status"], "failed")
            self.assertEqual(triage["evidenceScan"]["exit_code"], 1)
            self.assertEqual(triage["evidenceScan"]["checked_files"], 19)
            self.assertEqual(len(triage["runtimeResults"]), 3)

    def test_triage_accepts_closed_legacy_gate_prefix_with_unknown_browser_detail(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-legacy-gate-") as directory:
            root = Path(directory)
            self._archive(root)
            (root / "bundle/sources/runtime-structured").write_text(
                "cooking timestamp=2026-09-11T00:00:00Z "
                "event=browser-report-validation operation=browser-report-validation "
                "phase=evidence status=failed exit_code=1\n",
                encoding="utf-8",
            )
            results = TRIAGE.summarize(root / "bundle")["runtimeResults"]
            self.assertEqual(results[0]["status"], "failed")
            self.assertEqual(results[0]["exit_code"], 1)
            self.assertEqual(results[0]["report_state"], "unknown")
            self.assertEqual(results[0]["reason"], "legacy-unavailable")

    def test_collector_global_failure_is_typed_and_does_not_create_archive(self):
        """Fatal CLI validation emits safe metadata without an exception/path dump."""

        with tempfile.TemporaryDirectory(prefix="heph-gcp-collector-fatal-") as directory:
            root = Path(directory)
            output = root / "existing-PRIVATE_SECRET_VALUE"
            output.mkdir()
            archive = root / "should-not-be-created.tar.gz"
            result = subprocess.run(
                [
                    "python3",
                    str(ROOT / "collect-cooking-diagnostics.py"),
                    "--output-dir",
                    str(output),
                    "--archive",
                    str(archive),
                ],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 1)
            self.assertEqual(
                result.stderr.strip(),
                "HEPH_GCP_DIAGNOSTICS event=collector-failure operation=collection "
                "stage=validation reason_class=output-path-invalid status=failed exit_code=1",
            )
            self.assertNotIn("PRIVATE_SECRET_VALUE", result.stderr)
            self.assertFalse(archive.exists())
            self.assertTrue(output.is_dir())

    def test_collector_fatal_marker_survives_safe_projection_and_triage(self):
        """The serial-safe collector marker remains visible to the summary path."""

        with tempfile.TemporaryDirectory(prefix="heph-gcp-collector-marker-") as directory:
            root = Path(directory)
            serial = root / "serial.log"
            serial.write_text(
                "HEPH_GCP_DIAGNOSTICS event=collector-failure operation=collection "
                "stage=final-scan reason_class=final-scan status=failed exit_code=1\n",
                encoding="utf-8",
            )
            bundle = root / "bundle"
            self.assertEqual(
                COLLECTOR.collect(bundle, [f"serial={serial}"], None, None, None),
                0,
            )
            projected = (bundle / "sources" / "serial").read_text(encoding="utf-8")
            self.assertIn("stage=final-scan", projected)
            self.assertIn("reason_class=final-scan", projected)
            failures = TRIAGE.summarize(bundle)["failures"]
            self.assertTrue(
                any(
                    failure.get("operation") == "collection"
                    and failure.get("reason_class") == "final-scan"
                    and failure.get("status") == "failed"
                    for failure in failures
                ),
                failures,
            )

    def test_phase_timing_diagnostic_survives_collector_and_triage(self):
        """A fixed timing reason remains available after both safe filters."""

        with tempfile.TemporaryDirectory(prefix="heph-gcp-phase-timing-diagnostic-") as directory:
            root = Path(directory)
            source = root / "serial.log"
            source.write_text(
                "HEPH_GCP_DIAGNOSTICS event=phase-timing status=unavailable "
                "failed_stage=projection reason_class=missing-phase "
                "available_count=2 available_phases=dependency-setup,project-build "
                "missing_count=1 missing_phases=browser-setup\n",
                encoding="utf-8",
            )
            bundle = root / "bundle"
            self.assertEqual(COLLECTOR.collect(bundle, [f"serial={source}"], None, None, None), 0)
            projected = (bundle / "sources" / "serial").read_text(encoding="utf-8")
            self.assertEqual(projected, source.read_text(encoding="utf-8"))
            timing = TRIAGE.summarize(bundle)["phaseTiming"]
            self.assertEqual(
                timing,
                {
                    "status": "unavailable",
                    "failedStage": "projection",
                    "reasonClass": "missing-phase",
                    "availablePhases": ["dependency-setup", "project-build"],
                    "missingPhases": ["browser-setup"],
                    "availableCount": 2,
                    "missingCount": 1,
                },
            )

    def test_phase_timing_diagnostic_accepts_long_phase_lists(self):
        """The phase list is bounded by its enum, even when longer than a scalar value."""

        all_phases = list(COLLECTOR.PHASE_TIMING_PHASE_ORDER)
        self.assertGreater(len(",".join(all_phases)), 128)
        with tempfile.TemporaryDirectory(prefix="heph-gcp-phase-timing-long-list-") as directory:
            root = Path(directory)
            source = root / "serial.log"
            source.write_text(
                "HEPH_GCP_DIAGNOSTICS event=phase-timing status=unavailable "
                "failed_stage=projection reason_class=incomplete "
                f"available_count={len(all_phases)} available_phases={','.join(all_phases)} "
                "missing_count=0 missing_phases=none\n",
                encoding="utf-8",
            )
            bundle = root / "bundle"
            self.assertEqual(COLLECTOR.collect(bundle, [f"serial={source}"], None, None, None), 0)
            timing = TRIAGE.summarize(bundle)["phaseTiming"]
            self.assertEqual(timing["availableCount"], len(all_phases))
            self.assertEqual(timing["availablePhases"], all_phases)
            self.assertEqual(timing["missingPhases"], [])

    def test_phase_timing_diagnostic_rejects_unknown_payload_duplicate_and_count(self):
        """Both projection layers reject unallowlisted timing diagnostic fields."""

        malformed_markers = (
            "failed_stage=untrusted-stage",
            "payload=private",
            "available_count=999",
            "reason_class=missing-phase reason_class=unknown",
        )
        for malformed in malformed_markers:
            with self.subTest(malformed=malformed), tempfile.TemporaryDirectory(
                prefix="heph-gcp-phase-timing-invalid-"
            ) as directory:
                root = Path(directory)
                marker = (
                    "HEPH_GCP_DIAGNOSTICS event=phase-timing status=unavailable "
                    "failed_stage=projection reason_class=missing-phase "
                    "available_count=1 available_phases=project-build "
                    "missing_count=0 missing_phases=none\n"
                )
                if malformed.startswith("failed_stage="):
                    marker = marker.replace("failed_stage=projection", malformed)
                elif malformed.startswith("available_count="):
                    marker = marker.replace("available_count=1", malformed)
                else:
                    marker = marker.replace("none\n", f"none {malformed}\n")
                source = root / "serial.log"
                source.write_text(marker, encoding="utf-8")
                destination = root / "projected"
                with self.assertRaises(COLLECTOR.CollectionError):
                    COLLECTOR._project_text(source, destination)

                self._archive(root)
                (root / "bundle" / "sources" / "serial").write_text(marker, encoding="utf-8")
                with self.assertRaises(ValueError):
                    TRIAGE.summarize(root / "bundle")

    def test_timing_helper_failure_keeps_legacy_pair_reason_through_triage(self):
        """The shell wrapper's fixed reason remains compatible with triage."""

        with tempfile.TemporaryDirectory(prefix="heph-gcp-timing-helper-failure-") as directory:
            root = Path(directory)
            source = root / "runtime.log"
            source.write_text(
                "HEPH_GCP_COOKING event=timing-helper-error operation=cooking-workload "
                "phase=cooking stage=timing-helper-end reason_class=pair status=failed exit_code=2\n",
                encoding="utf-8",
            )
            bundle = root / "bundle"
            self.assertEqual(COLLECTOR.collect(bundle, [f"runtime-log={source}"], None, None, None), 0)
            projected = (bundle / "sources" / "runtime-log").read_text(encoding="utf-8")
            self.assertIn("reason_class=pair", projected)
            failures = TRIAGE.summarize(bundle)["failures"]
            self.assertTrue(
                any(
                    failure.get("reason_class") == "pair"
                    and failure.get("stage") == "timing-helper-end"
                    and failure.get("exit_code") == 2
                    for failure in failures
                ),
                failures,
            )

    def test_workload_budget_marker_keeps_safe_deadline_fields(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-workload-budget-") as directory:
            root = Path(directory)
            runtime = root / "runtime.log"
            runtime.write_text(
                "HEPH_GCP_COOKING event=workload-budget operation=cooking-workload "
                "phase=cooking status=failed exit_code=124 duration_ms=0 stage=deadline "
                "reason_class=insufficient-budget remaining_seconds=12 reserve_seconds=30\n"
                "HEPH_GCP_COOKING event=workload-budget operation=cooking-workload "
                "phase=cooking status=passed exit_code=0 duration_ms=180000 stage=allocated "
                "reason_class=none remaining_seconds=2400 reserve_seconds=300\n",
                encoding="utf-8",
            )
            bundle = root / "bundle"
            self.assertEqual(
                COLLECTOR.collect(bundle, [f"runtime-log={runtime}"], None, None, None),
                0,
            )
            projected = (bundle / "sources" / "runtime-log").read_text(encoding="utf-8")
            self.assertIn("duration_ms=0 stage=deadline", projected)
            self.assertIn("remaining_seconds=12 reserve_seconds=30", projected)
            self.assertIn("duration_ms=180000 stage=allocated", projected)
            failures = TRIAGE.summarize(bundle)["failures"]
            self.assertTrue(
                any(
                    failure.get("event") == "workload-budget"
                    and failure.get("duration_ms") == "0"
                    and failure.get("stage") == "deadline"
                    and failure.get("reason_class") == "insufficient-budget"
                    for failure in failures
                ),
                failures,
            )


if __name__ == "__main__":
    unittest.main()
