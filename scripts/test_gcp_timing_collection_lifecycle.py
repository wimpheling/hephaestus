#!/usr/bin/env python3
"""Execute the production timing/collection path with short lifecycle fixtures.

These are instrumentation correctness checks, never performance measurements.
The Rust timer and shell pipeline blocks are extracted from production sources;
only expensive workloads, systemd, deadlines and cloud transport are substituted.
"""

from pathlib import Path
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest


class TimingCollectionLifecycleTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "real Rust timer fixture requires rustc")
    def test_real_producer_through_actual_startup_collector_and_controller(self):
        with tempfile.TemporaryDirectory(prefix="heph-real-timer-lifecycle-") as directory:
            repo = Path(__file__).resolve().parent.parent
            root = Path(directory)
            source = (repo / "crates/hephaestus-app/tests/golden.rs").read_text()
            timer = source[
                source.index("const WORKLOAD_PHASE_TIMING_EVENT"):
                source.index("fn cooking_base_layout")
            ]
            # Exercise the exact timer implementation. All work here is a short
            # fixture, including names normally emitted by other Rust producers.
            phases = (
                "production-project-build", "runtime-guest-build", "runtime-worker-build",
                "oci-image-materialization", "gateway-services-ready", "gateway-readiness",
                "oci-builder", "oci-verifier", "golden-tests", "database-tests",
                "browser-initial", "browser-post-operation",
            )
            main = "fn main() {\n" + "".join(
                f'let timer = WorkloadPhaseTimer::start("{phase}", true); '
                "std::thread::sleep(std::time::Duration::from_millis(2)); "
                "timer.finish(true);\n" for phase in phases
            ) + "}\n"
            (root / "producer.rs").write_text("use std::time::Instant;\n" + timer + main)
            subprocess.run(
                ["rustc", "--edition=2021", str(root / "producer.rs"), "-o", str(root / "producer")],
                check=True,
            )
            (root / "input").mkdir()
            (root / "evidence/cooking").mkdir(parents=True)
            with (root / "input/serial.log").open("w") as markers:
                subprocess.run([str(root / "producer")], stderr=markers, check=True)
            runtime_spec = importlib.util.spec_from_file_location(
                "runtime_timing_fixture", repo / "scripts/test_gcp_supervisor_timing_runtime.py"
            )
            runtime = importlib.util.module_from_spec(runtime_spec)
            runtime_spec.loader.exec_module(runtime)
            result, runtime_root = runtime.GcpSupervisorTimingRuntimeTests()._run_cooking_caller(0)
            self.addCleanup(shutil.rmtree, runtime_root, True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            (root / "supervisor.jsonl").write_bytes((runtime_root / "supervisor.jsonl").read_bytes())
            helper = repo / "scripts/gcp_phase_timing.py"
            sha = "a" * 40
            image = "b" * 32
            env = {**os.environ, "HEPH_GCP_PHASE_TIMING_IMAGE_FINGERPRINT": image}
            for phase in ("dependency-setup", "browser-setup", "gateway-edge-ready"):
                domain = "workload-gateway" if phase == "gateway-edge-ready" else "workload"
                args = (
                    "--path", str(root / "evidence/cooking/phase-timing-workload.jsonl"),
                    "--phase", phase, "--trust", "workload", "--clock-domain", domain,
                    "--run-id", "98765", "--attempt", "2", "--source-sha", sha,
                )
                for command in (("start", *args), ("end", *args, "--outcome", "passed")):
                    subprocess.run(
                        [sys.executable, str(helper), *command],
                        check=True, capture_output=True, env=env,
                    )
            # Execute argument construction directly from both production
            # startup blocks and the controller, including their required phases.
            startup = (repo / "scripts/gcp-kvm-startup.sh").read_text()
            initial = startup[
                startup.index('  phase_timing_source="${cooking_evidence_root}'):
                startup.index('  snapshot_input="${cooking_evidence_root}')
            ]
            final = startup[
                startup.index('    phase_timing_sidecar="${temporary_root}'):
                startup.index('    phase_timing_sidecar_object_name="$(phase_timing_sidecar_object)')
            ]
            controller = (repo / "scripts/gcp-kvm-smoke.sh").read_text()
            validation_start = controller.index('python3 -B "$PHASE_TIMING_SCRIPT" validate-projection')
            validation = controller[validation_start:controller.index(" >/dev/null; then", validation_start)]
            script = f'''set -Eeuo pipefail
            cooking_evidence_root={root}/evidence/cooking
            supervisor_phase_timing_path={root}/supervisor.jsonl
            input_root={root}/input
            temporary_root={root}
            status_json={root}/status
            trusted_phase_timing_script={helper}
            run_id=98765
            run_attempt=2
            revision={sha}
            timing_image_fingerprint={image}
            test_mode=gcp-cooking
            workload_trust=trusted
            phase_timing_required_args=()
            phase_timing_projection_ready=false
            run_with_collection_deadline() {{ "$@"; }}
            phase_timing_emit_failure() {{ printf 'FAILED %s\\n' "$*"; return 1; }}
            {initial}
            [[ "$phase_timing_projection_ready" == true ]]
            '''
            for phase in ("archive", "evidence-scan", "upload"):
                script += f'''python3 "$trusted_phase_timing_script" start --path "$supervisor_phase_timing_path" --phase {phase} --trust supervisor --clock-domain guest-startup --run-id "$run_id" --attempt "$run_attempt" --source-sha "$revision" --image-fingerprint "$timing_image_fingerprint"
            python3 "$trusted_phase_timing_script" end --path "$supervisor_phase_timing_path" --phase {phase} --trust supervisor --clock-domain guest-startup --run-id "$run_id" --attempt "$run_attempt" --source-sha "$revision" --image-fingerprint "$timing_image_fingerprint" --outcome passed
            '''
            script += final + f'''
            PHASE_TIMING_SCRIPT={helper}
            phase_timing_staging="$phase_timing_sidecar"
            diagnostics_source_run_id="$run_id"
            diagnostics_source_attempt="$run_attempt"
            diagnostics_source_sha="$revision"
            {validation}
            python3 {repo}/scripts/collect-cooking-diagnostics.py --output-dir {root}/collected --source phase-timing="$phase_timing_input"
            '''
            (root / "lifecycle.sh").write_text(script)
            result = subprocess.run(
                ["bash", str(root / "lifecycle.sh")], text=True, capture_output=True, env=env,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            initial_value = json.loads((root / "input/phase-timing.json").read_text())
            final_value = json.loads((root / "phase-timing-final.json").read_text())
            production = next(
                item for item in final_value["phases"] if item["phase"] == "production-project-build"
            )
            self.assertEqual(production["measurement"], "informational")
            self.assertGreaterEqual(production["duration_ms"], 2)
            self.assertEqual(len(final_value["phases"]), len(initial_value["phases"]) + 3)

            # The controller reads the archived collector manifest through this
            # real CLI before accepting a measurement. A valid sidecar alone
            # does not prove that this post-cleanup reader accepts the bundle.
            bundle = root / "collected"
            summary_command = [
                sys.executable, str(repo / "scripts/summarize-cooking-diagnostics.py"),
                str(bundle),
            ]
            result = subprocess.run(summary_command, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            summary = json.loads(result.stdout)
            self.assertEqual(summary["sources"]["available"], ["phase-timing"])

            manifest_path = bundle / "manifest.json"
            manifest = json.loads(manifest_path.read_text())
            timing_record = next(record for record in manifest["sources"] if record["label"] == "phase-timing")
            retained_path = bundle / timing_record["path"]
            valid_timing = retained_path.read_text()
            wrong_domain = json.loads(valid_timing)
            wrong_domain["phases"][0]["clock_domain"] = "unknown-domain"
            for invalid in ("{", '{"malformed":true}', json.dumps(wrong_domain)):
                with self.subTest(invalid_timing=invalid[:40]):
                    retained_path.write_text(invalid)
                    result = subprocess.run(summary_command, text=True, capture_output=True)
                    self.assertEqual(result.returncode, 1)
                    self.assertIn("phase timing records are invalid", result.stderr)
            retained_path.write_text(valid_timing)
            timing_record["label"] = "unknown-source"
            manifest_path.write_text(json.dumps(manifest))
            result = subprocess.run(summary_command, text=True, capture_output=True)
            self.assertEqual(result.returncode, 1)
            self.assertIn("manifest source label is invalid", result.stderr)

            # A rejected timing source must still allow safe partial triage.
            invalid_source = root / "invalid-timing.json"
            invalid_source.write_text('{"malformed":true}')
            partial_bundle = root / "partial-collected"
            result = subprocess.run([
                sys.executable, str(repo / "scripts/collect-cooking-diagnostics.py"),
                "--output-dir", str(partial_bundle),
                "--source", f"phase-timing={invalid_source}",
                "--source", f"serial={root / 'input/serial.log'}",
            ], text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            result = subprocess.run(
                [*summary_command[:-1], str(partial_bundle)], text=True, capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            summary = json.loads(result.stdout)
            self.assertEqual(summary["collectionStatus"], "partial")
            self.assertEqual(summary["rejectedSources"][0]["label"], "phase-timing")


if __name__ == "__main__":
    unittest.main()
