#!/usr/bin/env python3

import importlib.util
import json
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("project-session-chat-negative-summary.py")
SPEC = importlib.util.spec_from_file_location("session_chat_negative_summary", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
PROJECTOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROJECTOR)


def valid_log() -> str:
    return "\n".join(
        [
            "running 1 test",
            "test bearer_push_starts_run_through_production_bootstrap ... ordinary test start",
            "HEPH_SESSION_CHAT_DENIAL_PROBE host=validated checks=10 refs=unchanged receives=unchanged",
            "ordinary stdout noise",
            "ordinary test after",
            "ok",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 35 filtered out; finished in 0.01s",
            "",
        ]
    )


class NegativeSummaryTests(unittest.TestCase):
    def write_log(self, directory: Path, content: str) -> Path:
        path = directory / "private-negative.log"
        path.write_text(content, encoding="utf-8")
        return path

    def test_projects_only_the_allowlisted_success(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            result = PROJECTOR.project(self.write_log(directory, valid_log()), 0)
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["validated_checks"], 10)
            self.assertEqual(result["golden_test_passes"], 1)
            self.assertNotIn("private-negative.log", json.dumps(result))

            output = directory / "summary.json"
            PROJECTOR.write_atomic(output, result)
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
            self.assertEqual(json.loads(output.read_text(encoding="utf-8")), result)

    def test_nonzero_runner_rejects_marker_even_when_golden_passed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result = PROJECTOR.project(self.write_log(Path(temporary), valid_log()), 34)
            self.assertEqual(result, {
                "schema": 1,
                "scenario": "session-chat-negative-capability",
                "status": "failed",
                "reason": "runner_nonzero",
                "runner_exit_status": 34,
            })

    def test_marker_shape_and_cardinality_are_strict(self) -> None:
        cases = {
            "missing": valid_log().replace(
                "HEPH_SESSION_CHAT_DENIAL_PROBE host=validated checks=10 refs=unchanged receives=unchanged\n",
                "",
            ),
            "duplicate": valid_log().replace(
                "HEPH_SESSION_CHAT_DENIAL_PROBE host=validated checks=10 refs=unchanged receives=unchanged\n",
                "HEPH_SESSION_CHAT_DENIAL_PROBE host=validated checks=10 refs=unchanged receives=unchanged\n"
                "HEPH_SESSION_CHAT_DENIAL_PROBE host=validated checks=10 refs=unchanged receives=unchanged\n",
            ),
            "malformed": valid_log().replace("checks=10", "checks=9"),
        }
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.assertEqual(
                PROJECTOR.project(self.write_log(directory, cases["missing"]), 0)["reason"],
                "marker_missing",
            )
            self.assertEqual(
                PROJECTOR.project(self.write_log(directory, cases["duplicate"]), 0)["reason"],
                "marker_duplicate",
            )
            self.assertEqual(
                PROJECTOR.project(self.write_log(directory, cases["malformed"]), 0)["reason"],
                "marker_malformed",
            )

            orphan = valid_log().replace("running 1 test\n", "").replace(
                "test result: ok.", "running 1 test\ntest result: ok."
            )
            self.assertEqual(
                PROJECTOR.project(self.write_log(directory, orphan), 0)["reason"],
                "marker_outside_harness",
            )

    def test_selected_test_must_pass_once_and_have_nonzero_suite(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            skipped = valid_log().replace(
                "test bearer_push_starts_run_through_production_bootstrap ... ordinary test start\n",
                "test bearer_push_starts_run_through_production_bootstrap ... ignored\n",
            )
            self.assertEqual(
                PROJECTOR.project(self.write_log(directory, skipped), 0)["reason"],
                "golden_test_skipped",
            )
            duplicate = valid_log().replace(
                "test bearer_push_starts_run_through_production_bootstrap ... ordinary test start\n",
                "test bearer_push_starts_run_through_production_bootstrap ... ordinary test start\n"
                "test bearer_push_starts_run_through_production_bootstrap ... ordinary test start\n",
            )
            self.assertEqual(
                PROJECTOR.project(self.write_log(directory, duplicate), 0)["reason"],
                "golden_test_duplicate",
            )
            completion_duplicate = valid_log().replace(
                "ok\ntest result:",
                "ok\nok\ntest result:",
            )
            self.assertEqual(
                PROJECTOR.project(self.write_log(directory, completion_duplicate), 0)["reason"],
                "golden_test_duplicate",
            )
            zero = valid_log().replace("running 1 test", "running 0 tests").replace(
                "test bearer_push_starts_run_through_production_bootstrap ...\n", ""
            )
            self.assertEqual(
                PROJECTOR.project(self.write_log(directory, zero), 0)["reason"],
                "golden_test_zero_tests",
            )
            broad = valid_log().replace("running 1 test", "running 36 tests").replace(
                "1 passed; 0 failed; 0 ignored", "35 passed; 0 failed; 1 ignored"
            )
            self.assertEqual(
                PROJECTOR.project(self.write_log(directory, broad), 0)["reason"],
                "golden_harness_wrong_count",
            )

    def test_truncation_and_hostile_private_lines_never_escape(self) -> None:
        secret = "PRIVATE_NEGATIVE_PROCESS_TOKEN_7e2d"
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            truncated = valid_log().rstrip("\n")
            result = PROJECTOR.project(self.write_log(directory, truncated), 0)
            self.assertEqual(result["reason"], "truncated_log")

            hostile = valid_log() + f"private diagnostic {secret}\n"
            log_path = self.write_log(directory, hostile)
            output = directory / "summary.json"
            self.assertEqual(
                PROJECTOR.main(
                    [
                        "--private-log",
                        str(log_path),
                        "--exit-status",
                        "0",
                        "--output",
                        str(output),
                    ]
                ),
                0,
            )
            self.assertNotIn(secret, output.read_text(encoding="utf-8"))

    @unittest.skipUnless(shutil.which("rustc"), "requires rustc for libtest output shape")
    def test_real_libtest_exact_nocapture_shape(self) -> None:
        source = """
#[test]
fn bearer_push_starts_run_through_production_bootstrap() {
    eprintln!("ordinary test start");
    eprintln!("HEPH_SESSION_CHAT_DENIAL_PROBE host=validated checks=10 refs=unchanged receives=unchanged");
    println!("ordinary stdout noise");
    eprintln!("ordinary test after");
}
"""
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source_path = directory / "shape.rs"
            binary_path = directory / "shape"
            source_path.write_text(source, encoding="utf-8")
            subprocess.run(
                ["rustc", "--test", str(source_path), "-o", str(binary_path)],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            completed = subprocess.run(
                [str(binary_path), "--exact", "--nocapture"],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
            )
            log_path = self.write_log(directory, completed.stdout.decode("utf-8"))
            self.assertEqual(completed.returncode, 0)
            self.assertEqual(PROJECTOR.project(log_path, completed.returncode)["status"], "passed")

    def test_ansi_and_diagnostic_noise_do_not_change_structural_result(self) -> None:
        noisy = valid_log().replace(
            "ordinary test after",
            "\x1b[32mordinary colored diagnostic\x1b[0m",
        )
        with tempfile.TemporaryDirectory() as temporary:
            result = PROJECTOR.project(self.write_log(Path(temporary), noisy), 0)
            self.assertEqual(result["status"], "passed")


if __name__ == "__main__":
    unittest.main()
