"""Regression tests for fail-closed disposable-VM cleanup."""

from __future__ import annotations

from pathlib import Path
import hashlib
import os
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
SMOKE = ROOT / "gcp-kvm-smoke.sh"


class GcpKvmSmokeCleanupTests(unittest.TestCase):
    def test_missing_vm_describe_error_does_not_use_unset_json_data(self) -> None:
        run_id = "34599999999"
        name = f"heph-kvm-smoke-{run_id}-1"
        with tempfile.TemporaryDirectory(prefix="heph-gcp-cleanup-") as raw:
            root = Path(raw)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            fake_gcloud = fake_bin / "gcloud"
            fake_gcloud.write_text(
                "#!/usr/bin/env bash\n"
                "set -eu\n"
                "printf \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/"
                f"{name}' was not found\\n\" >&2\n"
                "exit 1\n",
                encoding="utf-8",
            )
            fake_gcloud.chmod(0o700)
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}:{environment['PATH']}",
                    "GITHUB_RUN_ID": run_id,
                    "GITHUB_RUN_ATTEMPT": "1",
                    "GITHUB_SHA": "a" * 40,
                    "GCP_ZONE": "europe-west1-d",
                }
            )
            result = subprocess.run(
                [str(SMOKE), "cleanup"],
                cwd=ROOT.parent,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Disposable VM already absent (verified by describe)", result.stdout)
        self.assertNotIn("unbound variable", result.stderr)

    def test_diagnostic_failure_keeps_typed_marker_outside_recent_serial_tail(self) -> None:
        run_id = "34599999998"
        name = f"heph-kvm-smoke-{run_id}-1"
        secret = "HEPHAESTUS_BROWSER_SECRET_4d7ccf_org"
        serial = "\n".join(
            [
                "HEPH_GCP_KVM_STARTUP event=phase-start phase=diagnostic-bootstrap revision=" + "a" * 40,
                "gcp-kvm-startup: custom runner image Node executable cannot run as forge",
                "thread 'cooking::smoke' panicked at crates/foo/src/lib.rs:42:7: Permission denied",
                "error: Permission denied",
                "test cooking::smoke ... FAILED",
                "/opt/hephaestus/scripts/gcp-kvm-startup.sh: line 417: DIAGNOSTICS_OBJECT: unbound variable",
                f"fixture token={secret}",
                *[f"ordinary boot line {index}" for index in range(100)],
            "HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS FAIL test_result=failed",
            ]
        )
        with tempfile.TemporaryDirectory(prefix="heph-gcp-serial-context-") as raw:
            root = Path(raw)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            fake_gcloud = fake_bin / "gcloud"
            fake_gcloud.write_text(
                "#!/usr/bin/env bash\n"
                "set -eu\n"
                "case \" $* \" in\n"
                "  *' compute regions describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"INSTANCES\",\"limit\":\"2\",\"usage\":\"0\"}]}' ;;\n"
                "  *' instances describe '*) printf \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/"
                f"{name}' was not found\\n\" >&2; exit 1 ;;\n"
                "  *' instances create '*) exit 0 ;;\n"
                "  *' get-serial-port-output '*) printf '%s\\n' \"${GCP_FAKE_SERIAL}\" ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            fake_gcloud.chmod(0o700)
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}:{environment['PATH']}",
                    "GITHUB_RUN_ID": run_id,
                    "GITHUB_RUN_ATTEMPT": "1",
                    "GITHUB_SHA": "a" * 40,
                    "GCP_ZONE": "europe-west1-d",
                    "GCP_FAKE_SERIAL": serial,
                    "GCP_SMOKE_LOG": str(root / "smoke.log"),
                }
            )
            result = subprocess.run(
                [str(SMOKE), "diagnostic"],
                cwd=ROOT.parent,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Bounded typed serial failure context:", result.stderr)
        self.assertIn(
            "HEPH_GCP_RUNNER_IMAGE_READINESS tool=node class=node-not-runnable phase=runner-image-runtime",
            result.stderr,
        )
        self.assertIn("HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS FAIL test_result=failed", result.stderr)
        self.assertIn("HEPH_GCP_KVM_STARTUP event=phase-start phase=diagnostic-bootstrap", result.stderr)
        self.assertIn("HEPH_GCP_TEST test=rust-panic location=crates/foo/src/lib.rs:42:7", result.stderr)
        self.assertIn("HEPH_GCP_RUNTIME error=permission-denied errno=EACCES", result.stderr)
        self.assertIn("HEPH_GCP_TEST test=cooking::smoke status=failed", result.stderr)
        self.assertIn(
            "HEPH_GCP_SHELL error=unbound-variable source=gcp-kvm-startup.sh line=417 variable=DIAGNOSTICS_OBJECT",
            result.stderr,
        )
        self.assertNotIn(secret, result.stdout + result.stderr)
        self.assertNotIn("ordinary boot line 99", result.stderr)

    def test_smoke_create_attaches_private_diagnostics_identity_and_metadata(self) -> None:
        run_id = "34599999997"
        name = f"heph-kvm-smoke-{run_id}-1"
        serial = "HEPHAESTUS_GCP_KVM_SMOKE: FAIL phase=real-libkrun-smoke exit=1 revision=" + "a" * 40
        with tempfile.TemporaryDirectory(prefix="heph-gcp-smoke-metadata-") as raw:
            root = Path(raw)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            args_log = root / "gcloud-args.log"
            fake_gcloud = fake_bin / "gcloud"
            fake_gcloud.write_text(
                "#!/usr/bin/env bash\n"
                "set -eu\n"
                "printf '%s\\n' \"$*\" >>\"${GCP_ARGS_LOG}\"\n"
                "case \" $* \" in\n"
                "  *' compute regions describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"INSTANCES\",\"limit\":\"2\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' instances describe '*) printf \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/"
                f"{name}' was not found\\n\" >&2; exit 1 ;;\n"
                "  *' instances create '*) exit 0 ;;\n"
                "  *' get-serial-port-output '*) printf '%s\\n' \"${GCP_FAKE_SERIAL}\" ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            fake_gcloud.chmod(0o700)
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}:{environment['PATH']}",
                    "GCP_ARGS_LOG": str(args_log),
                    "GITHUB_RUN_ID": run_id,
                    "GITHUB_RUN_ATTEMPT": "1",
                    "GITHUB_SHA": "a" * 40,
                    "GCP_ZONE": "europe-west1-d",
                    "GCP_FAKE_SERIAL": serial,
                }
            )
            result = subprocess.run(
                [str(SMOKE), "smoke"],
                cwd=ROOT.parent,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )

            create_args = args_log.read_text(encoding="utf-8")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "--service-account=hephaestus-cooking-runtime@hephaestus-508000.iam.gserviceaccount.com",
            create_args,
        )
        self.assertIn("--scopes=storage-rw", create_args)
        self.assertIn("diagnostics-bucket=hephaestus-508000-cooking-diagnostics", create_args)
        self.assertIn(
            f"diagnostics-object=cooking/runs/{run_id}/1/{'a' * 40}.tar.gz",
            create_args,
        )
        self.assertIn("diagnostics-collector-script=", create_args)
        self.assertIn("diagnostics-scanner-script=", create_args)

    def test_new_run_gate_expectation_uses_mode_specific_script_hash(self) -> None:
        """The downloader's persisted contract names each producer correctly."""

        def run_mode(root: Path, mode: str) -> str:
            fake_bin = root / "bin"
            fake_bin.mkdir(parents=True)
            fake_gcloud = fake_bin / "gcloud"
            failure_marker = (
                "HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS FAIL test_result=failed"
                if mode == "diagnostic" else
                "HEPHAESTUS_GCP_COOKING: FAIL phase=test exit=1"
            )
            fake_gcloud.write_text(
                "#!/usr/bin/env bash\n"
                "set -eu\n"
                "case \" $* \" in\n"
                "  *' compute regions describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"INSTANCES\",\"limit\":\"2\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' storage objects describe '*) printf '%s\\n' '{\"name\":\"cooking/heph-gcp-cooking-cache.tar.zst\",\"size\":\"1783474345\",\"md5Hash\":\"di95x0b0Yqqt4RTyVUvb6A==\",\"generation\":\"1\"}' ;;\n"
                "  *' instances describe '*) echo \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/heph-kvm-smoke-34599999996-1' was not found\" >&2; exit 1 ;;\n"
                "  *' instances create '*) exit 0 ;;\n"
                f"  *' get-serial-port-output '*) printf '%s\\n' '{failure_marker}' ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            fake_gcloud.chmod(0o700)
            environment = {
                **os.environ,
                "PATH": f"{fake_bin}:{os.environ['PATH']}",
                "GITHUB_RUN_ID": "34599999996",
                "GITHUB_RUN_ATTEMPT": "1",
                "GITHUB_SHA": "a" * 40,
                "GCP_ZONE": "europe-west1-d",
                "RUNNER_TEMP": str(root),
            }
            result = subprocess.run(
                [str(SMOKE), mode], cwd=ROOT.parent, env=environment,
                text=True, capture_output=True, check=False,
            )
            if not (root / "gcp-diagnostics-gate-expectation").is_file():
                raise AssertionError(result.stdout + result.stderr)
            return (root / "gcp-diagnostics-gate-expectation").read_text(encoding="utf-8").strip()

        with tempfile.TemporaryDirectory(prefix="heph-gcp-gate-hashes-") as raw:
            root = Path(raw)
            cooking = run_mode(root / "cooking", "gcp-cooking")
            diagnostic = run_mode(root / "diagnostic", "diagnostic")
        helper_hash = hashlib.sha256((ROOT / "cooking-gate-results.py").read_bytes()).hexdigest()
        runtime_hash = hashlib.sha256((ROOT / "gcp-cooking-run.sh").read_bytes()).hexdigest()
        self.assertEqual(cooking, f"gcp-cooking {runtime_hash}")
        self.assertEqual(diagnostic, f"diagnostic {helper_hash}")
        self.assertNotEqual(cooking, diagnostic)

    def test_workflow_downloads_and_retains_smoke_diagnostics_after_cleanup(self) -> None:
        workflow = (ROOT.parent / ".github" / "workflows" / "cooking-e2e.yml").read_text(encoding="utf-8")
        self.assertGreaterEqual(workflow.count("inputs.cloud_mode == 'smoke'"), 3)
        self.assertIn("GCP_DIAGNOSTICS_ARCHIVE: ${{ runner.temp }}/gcp-diagnostics.tar.gz", workflow)
        self.assertIn("GCP_DIAGNOSTICS_STATUS: ${{ runner.temp }}/gcp-diagnostics-status.json", workflow)
        self.assertIn("name: Download and scan private diagnostics after VM deletion", workflow)
        self.assertIn("name: Retain safe diagnostics manifest", workflow)

    def test_workflow_exposes_no_vm_historical_diagnostics_triage(self) -> None:
        workflow = (ROOT.parent / ".github" / "workflows" / "cooking-e2e.yml").read_text(encoding="utf-8")
        self.assertIn("diagnostics-triage", workflow)
        self.assertIn("GCP_DIAGNOSTICS_SOURCE_RUN_ID", workflow)
        self.assertIn("GCP_DIAGNOSTICS_SOURCE_ATTEMPT", workflow)
        self.assertIn("GCP_DIAGNOSTICS_SOURCE_SHA", workflow)
        self.assertIn("GCP_DIAGNOSTICS_REQUIRE_SOURCE: 'true'", workflow)
        self.assertIn("Download and triage selected private diagnostics without a VM", workflow)
        self.assertIn("GH_TOKEN: ${{ github.token }}", workflow)


if __name__ == "__main__":
    unittest.main()
