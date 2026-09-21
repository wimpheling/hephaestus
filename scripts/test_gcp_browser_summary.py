#!/usr/bin/env python3
"""Focused tests for the private Playwright report projection contract."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PROJECTOR = load("browser_projector", ROOT / "project-playwright-browser-summary.py")
COLLECTOR = load("diagnostic_collector", ROOT / "collect-cooking-diagnostics.py")


def report(file_name: str, title: str, status: str, *, error_location: bool = True) -> dict:
    result = {
        "status": status,
        "duration": 12,
        "error": {
            "message": "expect(locator).toContainText()\nsecret=must-not-escape",
            "stack": "Error: secret=must-not-escape",
        },
    }
    if error_location:
        result["errorLocation"] = {
            "file": f"/repo/e2e/playwright/cooking-tests/{file_name}",
            "line": 106,
            "column": 7,
        }
    return {
        "config": {"rootDir": "/repo/e2e/playwright"},
        "suites": [
            {
                "file": f"cooking-tests/{file_name}",
                "line": 1,
                "column": 1,
                "title": file_name,
                "specs": [
                    {
                        "file": f"cooking-tests/{file_name}",
                        "line": 27,
                        "column": 1,
                        "title": title,
                        "tests": [{"results": [result]}],
                    }
                ],
            }
        ],
    }


def safe_session_report(
    *,
    test_id: str = PROJECTOR.SESSION_CHAT_TEST_ID,
    stages: tuple[str, ...] = PROJECTOR.SESSION_CHAT_STAGES,
    test_status: str = "passed",
    missing_stage: str | None = None,
    failed_stage: str | None = None,
    terminal_status: str | None = None,
) -> str:
    records = [{"event": "run_started", "test_count": 1}]
    for stage_id in stages:
        if stage_id == missing_stage:
            continue
        records.append({"event": "stage", "stage_id": stage_id, "status": "pending"})
        records.append(
            {"event": "stage", "stage_id": stage_id, "status": "failed" if stage_id == failed_stage else "passed"}
        )
        if stage_id == failed_stage:
            break
    records.append(
        {
            "event": "test",
            "test_id": test_id,
            "status": test_status,
            "duration_ms": 12,
            "retry": 0,
        }
    )
    counts = {"passed": 0, "failed": 0, "skipped": 0, "other": 0}
    if test_status == "passed":
        counts["passed"] = 1
    elif test_status == "failed":
        counts["failed"] = 1
    elif test_status == "skipped":
        counts["skipped"] = 1
    else:
        counts["other"] = 1
    records.append(
        {
            "event": "run_finished",
            "status": terminal_status or test_status,
            "counts": counts,
        }
    )
    return "".join(json.dumps(record, separators=(",", ":")) + "\n" for record in records)


class BrowserSummaryTests(unittest.TestCase):
    def test_projects_realistic_failure_without_raw_error_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            browser = root / "browser.initial"
            browser.mkdir()
            (browser / "playwright-report.json").write_text(
                json.dumps(
                    report(
                        "cooking-post-operation.spec.ts",
                        "cooking post-operation controls, provenance, recovery, and denial",
                        "failed",
                    )
                ),
                encoding="utf-8",
            )
            summary = PROJECTOR.project(root)
            self.assertEqual(summary["status"], "failed")
            self.assertEqual(summary["counts"], {"passed": 0, "failed": 1, "skipped": 0, "timed_out": 0})
            self.assertEqual(summary["failure_metadata"][0]["source_location_kind"], "error")
            self.assertEqual(summary["failure_metadata"][0]["source_line"], 106)
            self.assertNotIn("error", summary)
            self.assertNotIn("secret", json.dumps(summary))

    def test_multi_phase_reports_are_capped_and_retry_uses_terminal_result(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for index, (file_name, title, status) in enumerate(
                [
                    (
                        "cooking-live-review.spec.ts",
                        "cooking release install, mailbox, gateway configure, and binding",
                        "passed",
                    ),
                    (
                        "cooking-post-operation.spec.ts",
                        "cooking post-operation controls, provenance, recovery, and denial",
                        "timedOut",
                    ),
                ]
            ):
                browser = root / f"browser.{index}"
                browser.mkdir()
                value = report(file_name, title, status)
                value["suites"][0]["specs"][0]["tests"][0]["results"].append(
                    {"status": "passed", "duration": 1}
                )
                (browser / "playwright-report.json").write_text(json.dumps(value), encoding="utf-8")
            summary = PROJECTOR.project(root)
            self.assertEqual(summary["status"], "passed")
            self.assertEqual(summary["counts"]["passed"], 2)
            self.assertEqual(summary["passed_phases"], ["initial", "post-operation"])
            self.assertEqual(summary["failure_metadata"], [])

    def test_missing_and_malformed_reports_never_invent_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(PROJECTOR.project(root)["result_origin"], "no-browser-report")
            browser = root / "browser.bad"
            browser.mkdir()
            (browser / "playwright-report.json").write_text("{truncated", encoding="utf-8")
            summary = PROJECTOR.project(root)
            self.assertEqual(summary["status"], "unknown")
            self.assertEqual(summary["report_state"], "malformed")
            self.assertEqual(summary["failure_metadata"], [])

            partial_root = root / "partial-case"
            partial_root.mkdir()
            partial = partial_root / "browser.partial"
            partial.mkdir()
            (partial / "playwright.log").write_text("1 failed\n", encoding="utf-8")
            partial_summary = PROJECTOR.project(partial_root)
            self.assertEqual(partial_summary["status"], "unknown")
            self.assertEqual(partial_summary["report_state"], "partial")
            self.assertEqual(partial_summary["failure_metadata"], [])

    def test_top_level_reporter_error_is_unknown_without_projecting_its_text(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            browser = root / "browser.report-error"
            browser.mkdir()
            value = report(
                "cooking-post-operation.spec.ts",
                "cooking post-operation controls, provenance, recovery, and denial",
                "failed",
            )
            value["errors"] = [{"message": "secret payload must never be retained"}]
            (browser / "playwright-report.json").write_text(json.dumps(value), encoding="utf-8")
            summary = PROJECTOR.project(root)
            self.assertEqual(summary["status"], "unknown")
            self.assertEqual(summary["report_state"], "report-error")
            self.assertEqual(summary["failure_metadata"], [])
            self.assertNotIn("secret", json.dumps(summary))

    def test_aggregate_bounds_and_invalid_final_result_cannot_report_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            browser = root / "browser.valid"
            browser.mkdir()
            value = report(
                "cooking-live-review.spec.ts",
                "cooking release install, mailbox, gateway configure, and binding",
                "passed",
            )
            value["suites"][0]["specs"][0]["tests"][0]["results"].append({"status": "interrupted"})
            (browser / "playwright-report.json").write_text(json.dumps(value), encoding="utf-8")
            summary = PROJECTOR.project(root)
            self.assertEqual(summary["status"], "unknown")
            self.assertEqual(summary["report_state"], "partial")

            deep_root = root / "deep-case"
            deep_root.mkdir()
            deep_browser = deep_root / "browser.deep"
            deep_browser.mkdir()
            deep = report(
                "cooking-live-review.spec.ts",
                "cooking release install, mailbox, gateway configure, and binding",
                "passed",
            )
            cursor = deep["suites"][0]
            for _ in range(17):
                child = {"title": "nested", "specs": [], "suites": []}
                cursor["suites"] = [child]
                cursor = child
            (deep_browser / "playwright-report.json").write_text(json.dumps(deep), encoding="utf-8")
            deep_summary = PROJECTOR.project(deep_root)
            self.assertEqual(deep_summary["status"], "unknown")
            self.assertEqual(deep_summary["report_state"], "partial")

            many_root = root / "many-case"
            many_root.mkdir()
            for index in range(5):
                many_browser = many_root / f"browser.{index}"
                many_browser.mkdir()
                (many_browser / "playwright-report.json").write_text(json.dumps(value), encoding="utf-8")
            many_summary = PROJECTOR.project(many_root)
            self.assertEqual(many_summary["status"], "unknown")
            self.assertEqual(many_summary["report_state"], "truncated")

    def test_require_complete_journey_gate_has_distinct_success_and_missing_status(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for index, (file_name, title) in enumerate(
                [
                    (
                        "cooking-live-review.spec.ts",
                        "cooking release install, mailbox, gateway configure, and binding",
                    ),
                    (
                        "cooking-post-operation.spec.ts",
                        "cooking post-operation controls, provenance, recovery, and denial",
                    ),
                ]
            ):
                browser = root / f"browser.{index}"
                browser.mkdir()
                value = report(file_name, title, "passed")
                (browser / "playwright-report.json").write_text(json.dumps(value), encoding="utf-8")
            output = root / "complete-summary.json"
            complete = subprocess.run(
                [sys.executable, str(ROOT / "project-playwright-browser-summary.py"), str(root), str(output), "--require-complete-journey"],
                check=False,
            )
            self.assertEqual(complete.returncode, 0)
            self.assertEqual(json.loads(output.read_text())["observed_phases"], ["initial", "post-operation"])

            failed_root = root / "failed-case"
            failed_root.mkdir()
            for index, (file_name, title, status) in enumerate(
                [
                    (
                        "cooking-live-review.spec.ts",
                        "cooking release install, mailbox, gateway configure, and binding",
                        "passed",
                    ),
                    (
                        "cooking-post-operation.spec.ts",
                        "cooking post-operation controls, provenance, recovery, and denial",
                        "failed",
                    ),
                ]
            ):
                browser = failed_root / f"browser.{index}"
                browser.mkdir()
                (browser / "playwright-report.json").write_text(
                    json.dumps(report(file_name, title, status)), encoding="utf-8"
                )
            failed_output = failed_root / "summary.json"
            failed = subprocess.run(
                [sys.executable, str(ROOT / "project-playwright-browser-summary.py"), str(failed_root), str(failed_output), "--require-complete-journey"],
                check=False,
            )
            self.assertEqual(failed.returncode, 4)
            self.assertEqual(json.loads(failed_output.read_text())["status"], "failed")

            skipped_root = root / "skipped-case"
            skipped_root.mkdir()
            for index, (file_name, title, status) in enumerate(
                [
                    (
                        "cooking-live-review.spec.ts",
                        "cooking release install, mailbox, gateway configure, and binding",
                        "passed",
                    ),
                    (
                        "cooking-post-operation.spec.ts",
                        "cooking post-operation controls, provenance, recovery, and denial",
                        "skipped",
                    ),
                ]
            ):
                browser = skipped_root / f"browser.{index}"
                browser.mkdir()
                (browser / "playwright-report.json").write_text(
                    json.dumps(report(file_name, title, status)), encoding="utf-8"
                )
            skipped_output = skipped_root / "summary.json"
            skipped = subprocess.run(
                [sys.executable, str(ROOT / "project-playwright-browser-summary.py"), str(skipped_root), str(skipped_output), "--require-complete-journey"],
                check=False,
            )
            self.assertEqual(skipped.returncode, 4)
            self.assertEqual(json.loads(skipped_output.read_text())["passed_phases"], ["initial"])

            missing_root = root / "missing-case"
            missing_root.mkdir()
            (missing_root / "browser.initial").mkdir()
            (missing_root / "browser.initial" / "playwright-report.json").write_text(
                json.dumps(report(
                    "cooking-live-review.spec.ts",
                    "cooking release install, mailbox, gateway configure, and binding",
                    "passed",
                )),
                encoding="utf-8",
            )
            (missing_root / "browser.post-operation").mkdir()
            missing_output = missing_root / "summary.json"
            missing = subprocess.run(
                [sys.executable, str(ROOT / "project-playwright-browser-summary.py"), str(missing_root), str(missing_output), "--require-complete-journey"],
                check=False,
            )
            self.assertEqual(missing.returncode, 2)
            self.assertEqual(json.loads(missing_output.read_text())["report_state"], "partial")

    def test_session_chat_requires_initial_recovery_and_concurrency_journeys(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            initial = root / "browser.initial"
            recovery = root / "browser.recovery"
            initial.mkdir()
            recovery.mkdir()
            (initial / "playwright.log").write_text(safe_session_report(), encoding="utf-8")
            (recovery / "playwright.log").write_text(
                safe_session_report(
                    test_id=PROJECTOR.SESSION_CHAT_RECOVERY_TEST_ID,
                    stages=PROJECTOR.SESSION_CHAT_RECOVERY_STAGES,
                ),
                encoding="utf-8",
            )
            summary = PROJECTOR.project(root, "session-chat")
            self.assertEqual(summary["status"], "unknown")
            self.assertEqual(summary["result_origin"], "playwright-report")
            self.assertEqual(summary["observed_phases"], ["initial", "recovery"])
            self.assertEqual(summary["passed_phases"], ["initial", "recovery"])
            self.assertEqual(summary["counts"], {"passed": 2, "failed": 0, "skipped": 0, "timed_out": 0})

            output = root / "summary.json"
            complete = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "project-playwright-browser-summary.py"),
                    str(root),
                    str(output),
                    "--scenario",
                    "session-chat",
                    "--require-complete-journey",
                ],
                check=False,
            )
            self.assertEqual(complete.returncode, 2)
            projected = json.loads(output.read_text())
            self.assertEqual(projected["status"], "unknown")
            self.assertEqual(projected["report_state"], "partial")

            concurrency = root / "browser.concurrency"
            concurrency.mkdir()
            (concurrency / "playwright.log").write_text(
                safe_session_report(
                    test_id=PROJECTOR.SESSION_CHAT_CONCURRENT_TEST_ID,
                    stages=PROJECTOR.SESSION_CHAT_CONCURRENT_STAGES,
                ),
                encoding="utf-8",
            )
            summary = PROJECTOR.project(root, "session-chat")
            self.assertEqual(summary["status"], "passed")
            self.assertEqual(summary["report_state"], "complete")
            self.assertEqual(summary["observed_phases"], ["initial", "recovery", "concurrency"])
            self.assertEqual(summary["passed_phases"], ["initial", "recovery", "concurrency"])
            self.assertEqual(summary["counts"], {"passed": 3, "failed": 0, "skipped": 0, "timed_out": 0})

            complete = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "project-playwright-browser-summary.py"),
                    str(root),
                    str(output),
                    "--scenario",
                    "session-chat",
                    "--require-complete-journey",
                ],
                check=False,
            )
            self.assertEqual(complete.returncode, 0)
            projected = json.loads(output.read_text())
            self.assertEqual(projected["status"], "passed")
            self.assertEqual(projected["observed_phases"], ["initial", "recovery", "concurrency"])
            bundle = root / "bundle"
            self.assertEqual(
                COLLECTOR.collect(bundle, [f"browser-summary={output}"], None, None, None),
                0,
            )
            retained_path = bundle / "sources/browser-summary"
            self.assertEqual(stat.S_IMODE(retained_path.stat().st_mode), 0o600)
            retained = json.loads(retained_path.read_text(encoding="utf-8"))
            self.assertEqual(retained["observed_phases"], ["initial", "recovery", "concurrency"])

            invalid = root / "invalid-summary.json"
            projected["observed_phases"] = ["initial", "recovery", "unknown"]
            invalid.write_text(json.dumps(projected), encoding="utf-8")
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(root / "rejected", [f"browser-summary={invalid}"], None, None, None)

    def test_session_chat_missing_recovery_and_duplicate_or_wrong_id_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            initial = root / "browser.initial"
            initial.mkdir()
            (initial / "playwright.log").write_text(safe_session_report(), encoding="utf-8")
            missing = PROJECTOR.project(root, "session-chat")
            self.assertEqual(missing["status"], "unknown")
            self.assertEqual(missing["report_state"], "partial")
            self.assertEqual(missing["observed_phases"], ["initial"])

            duplicate = root / "browser.duplicate"
            duplicate.mkdir()
            (duplicate / "playwright.log").write_text(safe_session_report(), encoding="utf-8")
            duplicate_summary = PROJECTOR.project(root, "session-chat")
            self.assertEqual(duplicate_summary["status"], "unknown")
            self.assertEqual(duplicate_summary["report_state"], "partial")

            (duplicate / "playwright.log").write_text(
                safe_session_report(test_id="unknown_test"), encoding="utf-8"
            )
            wrong_id = PROJECTOR.project(root, "session-chat")
            self.assertEqual(wrong_id["status"], "unknown")
            self.assertEqual(wrong_id["report_state"], "partial")

    def test_session_chat_retains_initial_failure_or_timeout_without_recovery(self):
        for test_status, failed_stage, expected_status, expected_counts in (
            (
                "failed",
                "session_chat_second_response",
                "failed",
                {"passed": 0, "failed": 1, "skipped": 0, "timed_out": 0},
            ),
            (
                "timed_out",
                "session_chat_response",
                "timed_out",
                {"passed": 0, "failed": 0, "skipped": 0, "timed_out": 1},
            ),
        ):
            with self.subTest(test_status=test_status), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                initial = root / "browser.initial"
                initial.mkdir()
                terminal_status = "failed" if test_status == "timed_out" else None
                (initial / "playwright.log").write_text(
                    safe_session_report(
                        test_status=test_status,
                        failed_stage=failed_stage,
                        terminal_status=terminal_status,
                    ),
                    encoding="utf-8",
                )
                summary = PROJECTOR.project(root, "session-chat")
                self.assertEqual(summary["status"], expected_status)
                self.assertEqual(summary["report_state"], "partial")
                self.assertEqual(summary["observed_phases"], ["initial"])
                self.assertEqual(summary["passed_phases"], [])
                self.assertEqual(summary["counts"], expected_counts)
                output = root / "summary.json"
                complete = subprocess.run(
                    [
                        sys.executable,
                        str(ROOT / "project-playwright-browser-summary.py"),
                        str(root),
                        str(output),
                        "--scenario",
                        "session-chat",
                        "--require-complete-journey",
                    ],
                    check=False,
                )
                self.assertEqual(complete.returncode, 2)

    def test_session_chat_failed_timeout_retry_and_incomplete_recovery_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            initial = root / "browser.initial"
            recovery = root / "browser.recovery"
            initial.mkdir()
            recovery.mkdir()
            (initial / "playwright.log").write_text(safe_session_report(), encoding="utf-8")
            log = recovery / "playwright.log"
            recovery_kwargs = {
                "test_id": PROJECTOR.SESSION_CHAT_RECOVERY_TEST_ID,
                "stages": PROJECTOR.SESSION_CHAT_RECOVERY_STAGES,
            }
            log.write_text(
                safe_session_report(
                    **recovery_kwargs,
                    test_status="failed",
                    failed_stage="session_chat_response",
                ),
                encoding="utf-8",
            )
            failed = PROJECTOR.project(root, "session-chat")
            self.assertEqual(failed["status"], "failed")
            self.assertEqual(failed["passed_phases"], ["initial"])
            self.assertEqual(failed["counts"], {"passed": 1, "failed": 1, "skipped": 0, "timed_out": 0})

            # The reporter closes a timed-out test as a failed run with one
            # result in Playwright's `other` bucket.
            log.write_text(
                safe_session_report(
                    **recovery_kwargs,
                    test_status="timed_out",
                    failed_stage="session_chat_response",
                    terminal_status="failed",
                ),
                encoding="utf-8",
            )
            timed_out = PROJECTOR.project(root, "session-chat")
            self.assertEqual(timed_out["status"], "timed_out")
            self.assertEqual(timed_out["passed_phases"], ["initial"])
            self.assertEqual(timed_out["counts"]["timed_out"], 1)

            log.write_text(
                safe_session_report(**recovery_kwargs).replace('"test_count":1', '"test_count":true'),
                encoding="utf-8",
            )
            bool_count = PROJECTOR.project(root, "session-chat")
            self.assertEqual(bool_count["status"], "unknown")
            self.assertEqual(bool_count["report_state"], "partial")

            log.write_text(
                safe_session_report(**recovery_kwargs).replace('"retry":0', '"retry":1'),
                encoding="utf-8",
            )
            retry = PROJECTOR.project(root, "session-chat")
            self.assertEqual(retry["status"], "unknown")
            self.assertEqual(retry["report_state"], "partial")

            log.write_text(
                safe_session_report(
                    **recovery_kwargs,
                    missing_stage="session_chat_reconnect",
                ),
                encoding="utf-8",
            )
            incomplete = PROJECTOR.project(root, "session-chat")
            self.assertEqual(incomplete["status"], "unknown")
            self.assertEqual(incomplete["report_state"], "partial")

    def test_session_chat_failed_concurrency_remains_typed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for phase, test_id, stages in (
                ("initial", PROJECTOR.SESSION_CHAT_TEST_ID, PROJECTOR.SESSION_CHAT_STAGES),
                ("recovery", PROJECTOR.SESSION_CHAT_RECOVERY_TEST_ID, PROJECTOR.SESSION_CHAT_RECOVERY_STAGES),
                ("concurrency", PROJECTOR.SESSION_CHAT_CONCURRENT_TEST_ID, PROJECTOR.SESSION_CHAT_CONCURRENT_STAGES),
            ):
                browser = root / f"browser.{phase}"
                browser.mkdir()
                (browser / "playwright.log").write_text(
                    safe_session_report(
                        test_id=test_id,
                        stages=stages,
                        test_status="failed" if phase == "concurrency" else "passed",
                        failed_stage="session_chat_concurrent_stale_retry" if phase == "concurrency" else None,
                    ),
                    encoding="utf-8",
                )
            summary = PROJECTOR.project(root, "session-chat")
            self.assertEqual(summary["status"], "failed")
            self.assertEqual(summary["report_state"], "complete")
            self.assertEqual(summary["observed_phases"], ["initial", "recovery", "concurrency"])
            self.assertEqual(summary["passed_phases"], ["initial", "recovery"])
            self.assertEqual(summary["counts"], {"passed": 2, "failed": 1, "skipped": 0, "timed_out": 0})

    def test_collector_accepts_typed_projection_and_rejects_untrusted_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            browser = root / "browser.json"
            browser.write_text(
                json.dumps(
                    {
                        "status": "failed",
                        "suite": "cooking-playwright",
                        "test": "browser-journey",
                        "phase": "browser",
                        "component": "browser-e2e",
                        "result_origin": "playwright-report",
                        "report_state": "complete",
                        "counts": {"passed": 0, "failed": 1, "skipped": 0, "timed_out": 0},
                        "failure_metadata": [
                            {
                                "test_id": "cooking-post-operation",
                                "phase": "post-operation",
                                "status": "failed",
                                "error_class": "assertion",
                                "matcher": "toContainText",
                                "source_file": "e2e/playwright/cooking-tests/cooking-post-operation.spec.ts",
                                "source_line": 106,
                                "source_column": 7,
                                "source_location_kind": "error",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            output = root / "bundle"
            self.assertEqual(COLLECTOR.collect(output, [f"browser-summary={browser}"], None, None, None), 0)
            retained = json.loads((output / "sources/browser-summary").read_text(encoding="utf-8"))
            self.assertEqual(retained["failure_metadata"][0]["matcher"], "toContainText")
            malicious = json.loads(browser.read_text(encoding="utf-8"))
            malicious["failure_metadata"][0]["source_file"] = "/tmp/evil.spec.ts"
            browser.write_text(json.dumps(malicious), encoding="utf-8")
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(root / "rejected", [f"browser-summary={browser}"], None, None, None)
            malicious["failure_metadata"][0]["source_file"] = (
                "e2e/playwright/cooking-tests/cooking-post-operation.spec.ts"
            )
            malicious["error"] = "raw failure text"
            browser.write_text(json.dumps(malicious), encoding="utf-8")
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(root / "rejected-raw", [f"browser-summary={browser}"], None, None, None)

    def test_collector_accepts_typed_partial_failure_summary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            browser = root / "browser.json"
            browser.write_text(
                json.dumps(
                    {
                        "status": "failed",
                        "suite": "cooking-playwright",
                        "test": "browser-journey",
                        "phase": "browser",
                        "component": "browser-e2e",
                        "result_origin": "playwright-report",
                        "report_state": "partial",
                        "counts": {"passed": 0, "failed": 1, "skipped": 0, "timed_out": 0},
                        "observed_phases": ["initial"],
                        "passed_phases": [],
                        "failure_metadata": [],
                    }
                ),
                encoding="utf-8",
            )
            output = root / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"browser-summary={browser}"], None, None, None),
                0,
            )
            retained = json.loads(
                (output / "sources/browser-summary").read_text(encoding="utf-8")
            )
            self.assertEqual(retained["status"], "failed")
            self.assertEqual(retained["report_state"], "partial")


if __name__ == "__main__":
    unittest.main()
