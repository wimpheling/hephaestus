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
                "HEPH_GCP_DIAGNOSTICS event=collector-failure operation=collection "
                "stage=final-scan reason_class=final-scan status=failed exit_code=1",
                "HEPH_GCP_COOKING event=workload-budget operation=cooking-workload "
                "phase=cooking status=failed exit_code=124 duration_ms=0 stage=deadline "
                "reason_class=insufficient-budget remaining_seconds=12 reserve_seconds=30",
                "HEPH_GCP_COOKING event=workload-budget operation=cooking-workload "
                "phase=cooking status=passed exit_code=0 duration_ms=180000 stage=allocated "
                "reason_class=none remaining_seconds=2400 reserve_seconds=300",
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
        self.assertIn(
            "HEPH_GCP_DIAGNOSTICS event=collector-failure operation=collection "
            "stage=final-scan reason_class=final-scan status=failed exit_code=1",
            result.stderr,
        )
        self.assertIn(
            "HEPH_GCP_COOKING event=workload-budget operation=cooking-workload "
            "phase=cooking status=failed exit_code=124 duration_ms=0 stage=deadline "
            "reason_class=insufficient-budget remaining_seconds=12 reserve_seconds=30",
            result.stderr,
        )
        self.assertIn(
            "HEPH_GCP_COOKING event=workload-budget operation=cooking-workload "
            "phase=cooking status=passed exit_code=0 duration_ms=180000 stage=allocated "
            "reason_class=none remaining_seconds=2400 reserve_seconds=300",
            result.stderr,
        )
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
                "HEPHAESTUS_GCP_COOKING: FAIL phase=test exit=1 revision=" + "a" * 40
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

    def test_cooking_waits_for_outer_terminal_marker_after_workload_failure(self) -> None:
        """A workload marker cannot let cleanup kill the startup collector."""

        run_id = "34599999995"
        revision = "a" * 40
        name = f"heph-kvm-smoke-{run_id}-1"
        with tempfile.TemporaryDirectory(prefix="heph-gcp-terminal-order-") as raw:
            root = Path(raw)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            vm_state = root / "vm-created"
            state = root / "serial-count"
            fake_gcloud = fake_bin / "gcloud"
            fake_gcloud.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "case \" $* \" in\n"
                "  *' compute regions describe '*) printf '%s\\n' "
                "'{\"quotas\":[{\"metric\":\"INSTANCES\",\"limit\":\"2\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' storage objects describe '*) printf '%s\\n' "
                "'{\"name\":\"cooking/heph-gcp-cooking-cache.tar.zst\",\"size\":\"1783474345\",\"md5Hash\":\"di95x0b0Yqqt4RTyVUvb6A==\",\"generation\":\"1\"}' ;;\n"
                "  *' instances create '*) touch \"$GCP_VM_STATE\"; exit 0 ;;\n"
                "  *' instances describe '*)\n"
                "    if [[ -f \"$GCP_VM_STATE\" ]]; then printf '%s\\n' '{\"labels\":{\"purpose\":\"hephaestus-kvm-smoke\",\"run_id\":\"'\"$GITHUB_RUN_ID\"'\",\"run_attempt\":\"1\",\"sha\":\"'\"$GITHUB_SHA\"'\"}}'; exit 0; fi\n"
                "    printf \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/heph-kvm-smoke-%s-1' was not found\\n\" \"$GITHUB_RUN_ID\" >&2; exit 1 ;;\n"
                "  *' get-serial-port-output '*)\n"
                "    count=0; [[ -f \"$GCP_SERIAL_STATE\" ]] && count=$(<\"$GCP_SERIAL_STATE\")\n"
                "    count=$((count + 1)); printf '%s\\n' \"$count\" >\"$GCP_SERIAL_STATE\"\n"
                "    case \"$count\" in\n"
                "      1) printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: FAIL phase=cooking exit=1' ;;\n"
                "      2) printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: FAIL phase=cooking exit=1'; printf '%s\\n' 'HEPH_GCP_DIAGNOSTICS event=collection status=start object=gs://private/run.tar.gz' ;;\n"
                "      3) printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: FAIL phase=cooking exit=1'; printf '%s\\n' 'HEPH_GCP_DIAGNOSTICS event=upload status=pass object=gs://private/run.tar.gz' ;;\n"
                f"      *) printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: FAIL phase=cooking exit=1'; printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: FAIL phase=evidence exit=1 revision={revision}' ;;\n"
                + "    esac\n"
                "    exit 0 ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            fake_gcloud.chmod(0o700)
            fake_sleep = fake_bin / "sleep"
            fake_sleep.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            fake_sleep.chmod(0o700)
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}:{environment['PATH']}",
                    "GITHUB_RUN_ID": run_id,
                    "GITHUB_RUN_ATTEMPT": "1",
                    "GITHUB_SHA": revision,
                    "GCP_ZONE": "europe-west1-d",
                    "GCP_USE_STOCK_IMAGE": "true",
                    "GCP_SERIAL_STATE": str(state),
                    "GCP_VM_STATE": str(vm_state),
                }
            )
            result = subprocess.run(
                [str(SMOKE), "gcp-cooking"],
                cwd=ROOT.parent,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(state.read_text(encoding="utf-8").strip(), "4")
            self.assertIn("startup smoke reported failure", result.stderr)

    def test_cooking_waits_for_evidence_before_accepting_inner_success(self) -> None:
        """An inner PASS must leave time for the startup EXIT trap to upload."""

        run_id = "34599999994"
        revision = "b" * 40
        name = f"heph-kvm-smoke-{run_id}-1"
        with tempfile.TemporaryDirectory(prefix="heph-gcp-success-order-") as raw:
            root = Path(raw)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            vm_state = root / "vm-created"
            state = root / "serial-count"
            fake_gcloud = fake_bin / "gcloud"
            fake_gcloud.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "case \" $* \" in\n"
                "  *' compute regions describe '*) printf '%s\\n' "
                "'{\"quotas\":[{\"metric\":\"INSTANCES\",\"limit\":\"2\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' storage objects describe '*) printf '%s\\n' "
                "'{\"name\":\"cooking/heph-gcp-cooking-cache.tar.zst\",\"size\":\"1783474345\",\"md5Hash\":\"di95x0b0Yqqt4RTyVUvb6A==\",\"generation\":\"1\"}' ;;\n"
                "  *' instances create '*) touch \"$GCP_VM_STATE\"; exit 0 ;;\n"
                "  *' instances describe '*)\n"
                "    if [[ -f \"$GCP_VM_STATE\" ]]; then printf '%s\\n' '{\"labels\":{\"purpose\":\"hephaestus-kvm-smoke\",\"run_id\":\"'\"$GITHUB_RUN_ID\"'\",\"run_attempt\":\"1\",\"sha\":\"'\"$GITHUB_SHA\"'\"}}'; exit 0; fi\n"
                "    printf \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/heph-kvm-smoke-%s-1' was not found\\n\" \"$GITHUB_RUN_ID\" >&2; exit 1 ;;\n"
                "  *' get-serial-port-output '*)\n"
                "    count=0; [[ -f \"$GCP_SERIAL_STATE\" ]] && count=$(<\"$GCP_SERIAL_STATE\")\n"
                "    count=$((count + 1)); printf '%s\\n' \"$count\" >\"$GCP_SERIAL_STATE\"\n"
                "    case \"$count\" in\n"
                "      1|2) printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: PASS phase=cooking' ;;\n"
                "      3) printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: PASS phase=cooking'; printf '%s\\n' 'HEPH_GCP_DIAGNOSTICS event=upload status=pass object=gs://private/run.tar.gz'; printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: PASS' ;;\n"
                "      *) printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: PASS' ;;\n"
                "    esac\n"
                "    exit 0 ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            fake_gcloud.chmod(0o700)
            fake_sleep = fake_bin / "sleep"
            fake_sleep.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            fake_sleep.chmod(0o700)
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}:{environment['PATH']}",
                    "GITHUB_RUN_ID": run_id,
                    "GITHUB_RUN_ATTEMPT": "1",
                    "GITHUB_SHA": revision,
                    "GCP_ZONE": "europe-west1-d",
                    "GCP_USE_STOCK_IMAGE": "true",
                    "GCP_SERIAL_STATE": str(state),
                    "GCP_VM_STATE": str(vm_state),
                }
            )
            result = subprocess.run(
                [str(SMOKE), "gcp-cooking"],
                cwd=ROOT.parent,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(state.read_text(encoding="utf-8").strip(), "3")
            self.assertIn("GCE gcp-cooking passed", result.stdout)

    def test_cooking_waits_through_timeout_and_collection_failure(self) -> None:
        """Timeout and evidence failure still require the outer final marker."""

        run_id = "34599999993"
        revision = "c" * 40
        with tempfile.TemporaryDirectory(prefix="heph-gcp-timeout-order-") as raw:
            root = Path(raw)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            vm_state = root / "vm-created"
            serial_state = root / "serial-count"
            fake_gcloud = fake_bin / "gcloud"
            fake_gcloud.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "case \" $* \" in\n"
                "  *' compute regions describe '*) printf '%s\\n' "
                "'{\"quotas\":[{\"metric\":\"INSTANCES\",\"limit\":\"2\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' storage objects describe '*) printf '%s\\n' "
                "'{\"name\":\"cooking/heph-gcp-cooking-cache.tar.zst\",\"size\":\"1783474345\",\"md5Hash\":\"di95x0b0Yqqt4RTyVUvb6A==\",\"generation\":\"1\"}' ;;\n"
                "  *' instances create '*) touch \"$GCP_VM_STATE\"; exit 0 ;;\n"
                "  *' instances describe '*)\n"
                "    if [[ -f \"$GCP_VM_STATE\" ]]; then exit 0; fi\n"
                "    printf \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/heph-kvm-smoke-%s-1' was not found\\n\" \"$GITHUB_RUN_ID\" >&2; exit 1 ;;\n"
                "  *' get-serial-port-output '*)\n"
                "    count=0; [[ -f \"$GCP_SERIAL_STATE\" ]] && count=$(<\"$GCP_SERIAL_STATE\")\n"
                "    count=$((count + 1)); printf '%s\\n' \"$count\" >\"$GCP_SERIAL_STATE\"\n"
                "    case \"$count\" in\n"
                "      1) printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: FAIL phase=cooking exit=124' ;;\n"
                "      2) printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: FAIL phase=cooking exit=124'; printf '%s\\n' 'HEPH_GCP_DIAGNOSTICS event=scan status=fail reason=archive-invalid' ;;\n"
                f"      *) printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: FAIL phase=evidence exit=124 revision={revision}' ;;\n"
                "    esac\n"
                "    exit 0 ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            fake_gcloud.chmod(0o700)
            fake_sleep = fake_bin / "sleep"
            fake_sleep.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            fake_sleep.chmod(0o700)
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}:{environment['PATH']}",
                    "GITHUB_RUN_ID": run_id,
                    "GITHUB_RUN_ATTEMPT": "1",
                    "GITHUB_SHA": revision,
                    "GCP_ZONE": "europe-west1-d",
                    "GCP_USE_STOCK_IMAGE": "true",
                    "GCP_SERIAL_STATE": str(serial_state),
                    "GCP_VM_STATE": str(vm_state),
                }
            )
            result = subprocess.run(
                [str(SMOKE), "gcp-cooking"],
                cwd=ROOT.parent,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(serial_state.read_text(encoding="utf-8").strip(), "3")
            self.assertIn("startup smoke reported failure", result.stderr)

    def test_cooking_poll_deadline_exits_without_outer_marker_and_cleanup_remains_available(self) -> None:
        """An absent outer marker must hit the bound while leaving cleanup usable."""

        run_id = "34599999992"
        revision = "d" * 40
        with tempfile.TemporaryDirectory(prefix="heph-gcp-poll-deadline-") as raw:
            root = Path(raw)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            vm_state = root / "vm-created"
            date_state = root / "date-count"
            serial_state = root / "serial-count"
            fake_gcloud = fake_bin / "gcloud"
            fake_gcloud.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "case \" $* \" in\n"
                "  *' compute regions describe '*) printf '%s\\n' "
                "'{\"quotas\":[{\"metric\":\"INSTANCES\",\"limit\":\"2\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' storage objects describe '*) printf '%s\\n' "
                "'{\"name\":\"cooking/heph-gcp-cooking-cache.tar.zst\",\"size\":\"1783474345\",\"md5Hash\":\"di95x0b0Yqqt4RTyVUvb6A==\",\"generation\":\"1\"}' ;;\n"
                "  *' instances create '*) touch \"$GCP_VM_STATE\"; exit 0 ;;\n"
                "  *' instances describe '*)\n"
                "    if [[ -f \"$GCP_VM_STATE\" ]]; then printf '%s\\n' '{\"labels\":{\"purpose\":\"hephaestus-kvm-smoke\",\"run_id\":\"'\"$GITHUB_RUN_ID\"'\",\"run_attempt\":\"1\",\"sha\":\"'\"$GITHUB_SHA\"'\"}}'; exit 0; fi\n"
                "    printf \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/heph-kvm-smoke-%s-1' was not found\\n\" \"$GITHUB_RUN_ID\" >&2; exit 1 ;;\n"
                "  *' instances delete '*) rm -f \"$GCP_VM_STATE\"; exit 0 ;;\n"
                "  *' get-serial-port-output '*)\n"
                "    count=0; [[ -f \"$GCP_SERIAL_STATE\" ]] && count=$(<\"$GCP_SERIAL_STATE\")\n"
                "    count=$((count + 1)); printf '%s\\n' \"$count\" >\"$GCP_SERIAL_STATE\"\n"
                "    printf '%s\\n' 'HEPHAESTUS_GCP_COOKING: FAIL phase=cooking exit=1'\n"
                "    exit 0 ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            fake_gcloud.chmod(0o700)
            fake_date = fake_bin / "date"
            fake_date.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "if [[ \"${1:-}\" != +%s ]]; then exec /usr/bin/date \"$@\"; fi\n"
                "count=0; [[ -f \"$GCP_DATE_STATE\" ]] && count=$(<\"$GCP_DATE_STATE\")\n"
                "count=$((count + 1)); printf '%s\\n' \"$count\" >\"$GCP_DATE_STATE\"\n"
                "if ((count == 1)); then printf '%s\\n' 1000; elif ((count == 2)); then printf '%s\\n' 1001; else printf '%s\\n' 3401; fi\n",
                encoding="utf-8",
            )
            fake_date.chmod(0o700)
            fake_sleep = fake_bin / "sleep"
            fake_sleep.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            fake_sleep.chmod(0o700)
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}:{environment['PATH']}",
                    "GITHUB_RUN_ID": run_id,
                    "GITHUB_RUN_ATTEMPT": "1",
                    "GITHUB_SHA": revision,
                    "GCP_ZONE": "europe-west1-d",
                    "GCP_SERIAL_STATE": str(serial_state),
                    "GCP_DATE_STATE": str(date_state),
                    "GCP_VM_STATE": str(vm_state),
                }
            )
            result = subprocess.run(
                [str(SMOKE), "gcp-cooking"],
                cwd=ROOT.parent,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("timed out waiting for the startup gcp-cooking marker", result.stderr)
            self.assertEqual(serial_state.read_text(encoding="utf-8").strip(), "1")
            self.assertFalse(vm_state.exists())

            cleanup = subprocess.run(
                [str(SMOKE), "cleanup"],
                cwd=ROOT.parent,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertEqual(cleanup.returncode, 0, cleanup.stdout + cleanup.stderr)
            self.assertIn("Disposable VM already absent", cleanup.stdout)
            self.assertFalse(vm_state.exists())

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
        self.assertIn("GCP_DIAGNOSTICS_CONTROLLER_SHA", workflow)
        self.assertIn("diagnostics_workload_sha", workflow)
        self.assertIn("GCP_DIAGNOSTICS_REQUIRE_SOURCE: 'true'", workflow)
        self.assertIn("Download and triage selected private diagnostics without a VM", workflow)
        self.assertIn("GH_TOKEN: ${{ github.token }}", workflow)


if __name__ == "__main__":
    unittest.main()
