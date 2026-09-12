"""Regression coverage for the bounded Cooking workload failure markers."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
RUNNER = ROOT.parent / "examples" / "cooking" / "run.sh"


class CookingRunWorkloadMarkerTests(unittest.TestCase):
    def run_fixture(
        self,
        *,
        preflight_status: int = 0,
        cargo_status: int = 0,
        gateway_status: int = 0,
        timing_helper_status: int | None = None,
        timing_helper_failure_phase: str | None = None,
        timing_enabled: bool = False,
        timing_run_id: str | None = None,
        timing_attempt: int = 1,
        timing_output: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory(prefix="heph-cooking-run-marker-") as raw:
            root = Path(raw)
            repo = root / "repo"
            cooking = repo / "examples" / "cooking"
            gateway = cooking / "cooking-gateway"
            scripts = repo / "scripts"
            fake_bin = root / "bin"
            diagnostics = root / "diagnostics"
            temporary = root / "tmp"
            gateway.mkdir(parents=True)
            scripts.mkdir(parents=True)
            fake_bin.mkdir()
            diagnostics.mkdir()
            temporary.mkdir()
            shutil.copy2(RUNNER, cooking / "run.sh")
            for name in ("shell-failure-diagnostics.sh", "check-browser-evidence.py", "gcp_phase_timing.py"):
                shutil.copy2(ROOT.parent / "scripts" / name, scripts / name)
            if timing_helper_status is not None:
                (scripts / "gcp_phase_timing.py").write_text(
                    f"#!/usr/bin/env python3\nraise SystemExit({timing_helper_status})\n",
                    encoding="utf-8",
                )
            elif timing_helper_failure_phase is not None:
                (scripts / "gcp_phase_timing.py").write_text(
                    "#!/usr/bin/env python3\n"
                    "import sys\n"
                    f"raise SystemExit(31 if len(sys.argv) > 1 and sys.argv[1] == {timing_helper_failure_phase!r} else 0)\n",
                    encoding="utf-8",
                )
            for path, status in (
                (cooking / "preflight.sh", preflight_status),
                (scripts / "run-gateway-libkrun-e2e.sh", gateway_status),
            ):
                path.write_text(f"#!/usr/bin/env bash\nexit {status}\n", encoding="utf-8")
                path.chmod(0o700)
            (fake_bin / "rustup").write_text(
                f"#!/usr/bin/env bash\nexit {cargo_status}\n", encoding="utf-8"
            )
            (fake_bin / "cargo").write_text(
                f"#!/usr/bin/env bash\nexit {cargo_status}\n", encoding="utf-8"
            )
            (fake_bin / "rustup").chmod(0o700)
            (fake_bin / "cargo").chmod(0o700)
            environment = {
                **os.environ,
                "PATH": f"{fake_bin}:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                "HEPHAESTUS_COOKING_BROWSER_E2E": "0",
                "HEPHAESTUS_COOKING_DIAGNOSTICS_DIR": str(diagnostics),
                "HEPHAESTUS_LIBKRUN_TMP_ROOT": str(temporary),
                "TMPDIR": str(temporary),
                "HEPHAESTUS_COOKING_TIMEOUT_SECONDS": "30",
                "HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE": f"ubuntu@sha256:{'a' * 64}",
                "HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE": f"ubuntu@sha256:{'b' * 64}",
                "GITHUB_RUN_ID": "123",
                "GITHUB_RUN_ATTEMPT": "1",
            }
            if timing_run_id is not None:
                environment.pop("GITHUB_RUN_ID")
                environment.pop("GITHUB_RUN_ATTEMPT")
                environment["HEPH_GCP_PHASE_TIMING_RUN_ID"] = timing_run_id
                environment["HEPH_GCP_PHASE_TIMING_ATTEMPT"] = str(timing_attempt)
            if timing_enabled or timing_helper_status is not None or timing_helper_failure_phase is not None or timing_run_id is not None:
                timing_path = root / "phase-timing.jsonl"
                environment.update(
                    HEPH_GCP_PHASE_TIMING_PATH=str(timing_path),
                    HEPH_GCP_PHASE_TIMING_SOURCE_SHA="a" * 40,
                )
            result = subprocess.run(
                [str(cooking / "run.sh")],
                cwd=repo,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )
            if timing_output is not None and (root / "phase-timing.jsonl").exists():
                timing_output.write_bytes((root / "phase-timing.jsonl").read_bytes())
            return result

    def test_dependency_setup_failure_is_typed(self) -> None:
        result = self.run_fixture(preflight_status=17)
        self.assertEqual(result.returncode, 17, result.stdout + result.stderr)
        self.assertIn(
            "HEPH_GCP_COOKING event=workload-step operation=cooking-workload "
            "phase=cooking stage=dependency-setup status=failed exit_code=17",
            result.stdout,
        )
        self.assertNotIn("stage=project-build status=start", result.stdout)
        self.assertIn("stage=preflight-command-checks status=failed exit_code=17", result.stdout)

    def test_project_build_failure_is_typed(self) -> None:
        result = self.run_fixture(cargo_status=19)
        self.assertEqual(result.returncode, 19, result.stdout + result.stderr)
        self.assertIn("stage=project-build status=failed exit_code=19", result.stdout)
        self.assertIn("stage=rustup-target status=failed exit_code=19", result.stdout)
        self.assertNotIn("stage=gateway-e2e status=start", result.stdout)

    def test_gateway_failure_is_typed(self) -> None:
        result = self.run_fixture(gateway_status=23)
        self.assertEqual(result.returncode, 23, result.stdout + result.stderr)
        self.assertIn("stage=gateway-e2e status=failed exit_code=23", result.stdout)
        self.assertIn("stage=gateway-invocation status=failed exit_code=23", result.stdout)

    def test_timing_helper_failure_is_typed_before_dependency_command(self) -> None:
        result = self.run_fixture(timing_helper_status=31)
        self.assertEqual(result.returncode, 31, result.stdout + result.stderr)
        self.assertIn("stage=dependency-setup status=start", result.stdout)
        self.assertIn("stage=dependency-setup status=failed exit_code=31", result.stdout)
        self.assertIn("stage=timing-helper-start status=failed exit_code=31", result.stdout)
        self.assertNotIn("stage=project-build status=start", result.stdout)

    def test_timing_helper_end_failure_is_typed(self) -> None:
        result = self.run_fixture(timing_helper_failure_phase="end")
        self.assertEqual(result.returncode, 31, result.stdout + result.stderr)
        self.assertIn("stage=timing-helper-end status=failed exit_code=31", result.stdout)
        self.assertNotIn("stage=project-build status=passed", result.stdout)

    def test_timing_records_preserve_supervisor_run_identity(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-cooking-run-timing-") as directory:
            timing_output = Path(directory) / "phase-timing.jsonl"
            result = self.run_fixture(
                timing_run_id="98765",
                timing_attempt=2,
                timing_output=timing_output,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            records = [json.loads(line) for line in timing_output.read_text(encoding="utf-8").splitlines()]
            self.assertGreaterEqual(len(records), 4)
            self.assertTrue(all(record["run_id"] == "98765" for record in records), records)
            self.assertTrue(all(record["attempt"] == 2 for record in records), records)

    def test_success_reports_each_bounded_workload_stage(self) -> None:
        result = self.run_fixture()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for stage in ("dependency-setup", "project-build", "gateway-e2e"):
            self.assertIn(f"stage={stage} status=passed", result.stdout)

    def test_success_with_real_timing_helper_skips_marker_only_gateway_stage(self) -> None:
        result = self.run_fixture(timing_enabled=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("stage=gateway-e2e status=passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
