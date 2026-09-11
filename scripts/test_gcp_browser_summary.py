#!/usr/bin/env python3
"""Focused tests for the private Playwright report projection contract."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
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


if __name__ == "__main__":
    unittest.main()
