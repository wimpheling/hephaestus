"""Regression tests for fail-closed disposable-VM cleanup."""

from __future__ import annotations

from pathlib import Path
import hashlib
import json
import os
import subprocess
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).parent
SMOKE = ROOT / "gcp-kvm-smoke.sh"


class GcpKvmSmokeCleanupTests(unittest.TestCase):
    @staticmethod
    def _session_chat_archive(root: Path, *, passed: bool) -> Path:
        sources = root / "archive-sources"
        output = root / ("bundle-passed" if passed else "bundle-failed")
        archive = root / ("passed.tar.gz" if passed else "failed.tar.gz")
        sources.mkdir(exist_ok=True)
        summary = (
            {
                "schema": 1,
                "scenario": "session-chat-negative-capability",
                "status": "passed",
                "reason": "validated",
                "validated_checks": 10,
                "refs": "unchanged",
                "receives": "unchanged",
                "golden_test": "bearer_push_starts_run_through_production_bootstrap",
                "golden_test_passes": 1,
                "runner_exit_status": 0,
            }
            if passed
            else {
                "schema": 1,
                "scenario": "session-chat-negative-capability",
                "status": "failed",
                "reason": "golden_test_failed",
                "runner_exit_status": 17,
            }
        )
        files = {
            "serial": "HEPH_GCP_COOKING event=workload-result phase=cooking status=passed\n",
            "host-journal": "host journal unavailable\n",
            "runtime-structured": "HEPH_GCP_DIAGNOSTICS event=collection status=pass\n",
            "browser-summary": '{"status":"passed","phase":"browser","test":"browser-report","exit_code":0}\n',
            "session-chat-negative-summary": json.dumps(summary, separators=(",", ":")) + "\n",
        }
        arguments = [
            "python3",
            str(ROOT / "collect-cooking-diagnostics.py"),
            "--output-dir",
            str(output),
        ]
        for label, content in files.items():
            source = sources / label
            source.write_text(content, encoding="utf-8")
            arguments += ["--source", f"{label}={source}"]
        arguments += ["--archive", str(archive)]
        result = subprocess.run(arguments, text=True, capture_output=True, check=False)
        if result.returncode != 0:
            raise AssertionError(result.stdout + result.stderr)
        return archive

    def _run_session_chat_download(self, root: Path, archive: Path) -> tuple[subprocess.CompletedProcess[str], dict[str, object]]:
        fake_bin = root / "bin"
        fake_bin.mkdir(exist_ok=True)
        fake_gcloud = fake_bin / "gcloud"
        fake_gcloud.write_text(
            "#!/usr/bin/env bash\n"
            "set -Eeuo pipefail\n"
            "case \" $* \" in\n"
            "  *' instances describe '*) printf \"The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/heph-kvm-smoke-%s-%s' was not found\\n\" \"$GITHUB_RUN_ID\" \"$GITHUB_RUN_ATTEMPT\" >&2; exit 1 ;;\n"
            "  *' storage cp '*) cp \"$GCP_FIXTURE_ARCHIVE\" \"$4\"; exit 0 ;;\n"
            "  *) exit 2 ;;\n"
            "esac\n",
            encoding="utf-8",
        )
        fake_gcloud.chmod(0o700)
        destination = root / "downloaded.tar.gz"
        status_path = root / "status.json"
        environment = {
            **os.environ,
            "PATH": f"{fake_bin}:{os.environ['PATH']}",
            "GCP_FIXTURE_ARCHIVE": str(archive),
            "GCP_COOKING_SCENARIO": "session-chat",
            "GITHUB_RUN_ID": "34599999991",
            "GITHUB_RUN_ATTEMPT": "1",
            "GITHUB_SHA": "a" * 40,
            "GCP_ZONE": "europe-west1-d",
            "GCP_DIAGNOSTICS_ARCHIVE": str(destination),
            "GCP_DIAGNOSTICS_STATUS": str(status_path),
            "RUNNER_TEMP": str(root),
        }
        result = subprocess.run(
            [str(SMOKE), "download-diagnostics"],
            cwd=ROOT.parent,
            env=environment,
            text=True,
            capture_output=True,
            check=False,
        )
        return result, json.loads(status_path.read_text(encoding="utf-8"))

    @staticmethod
    def _rewrite_session_chat_archive(root: Path, archive: Path, mutation: str) -> Path:
        unpacked = root / f"unpacked-{mutation}"
        unpacked.mkdir()
        with tarfile.open(archive, "r:gz") as source:
            source.extractall(unpacked)
        bundle = unpacked / "cooking-diagnostics"
        manifest_path = bundle / "manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        summary_path = bundle / "sources" / "session-chat-negative-summary"
        if mutation == "boolean-schema":
            summary = json.loads(summary_path.read_text(encoding="utf-8"))
            summary["schema"] = True
            summary_path.write_text(json.dumps(summary, separators=(",", ":")) + "\n", encoding="utf-8")
        elif mutation == "duplicate-status":
            summary_text = summary_path.read_text(encoding="utf-8")
            summary_path.write_text(summary_text.replace('"status":"passed"', '"status":"passed","status":"failed"', 1), encoding="utf-8")
        elif mutation == "duplicate-source":
            manifest["sources"].append(dict(next(record for record in manifest["sources"] if record["label"] == "session-chat-negative-summary")))
        else:
            raise AssertionError(f"unknown archive mutation: {mutation}")
        for record in manifest["sources"]:
            if record["label"] == "session-chat-negative-summary":
                record["bytes"] = summary_path.stat().st_size
                record["sha256"] = hashlib.sha256(summary_path.read_bytes()).hexdigest()
        manifest_path.write_text(json.dumps(manifest, separators=(",", ":")) + "\n", encoding="utf-8")
        mutated = root / f"{mutation}.tar.gz"
        with tarfile.open(mutated, "w:gz") as target:
            target.add(bundle, arcname="cooking-diagnostics")
        return mutated

    def test_session_chat_download_accepts_only_typed_passed_negative_summary(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-gcp-session-negative-gate-") as raw:
            root = Path(raw)
            passed = self._session_chat_archive(root, passed=True)
            result, status = self._run_session_chat_download(root, passed)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(status["sessionChatNegative"], "passed")

            failed = self._session_chat_archive(root, passed=False)
            result, status = self._run_session_chat_download(root, failed)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(status["sessionChatNegative"], "failed")
            self.assertEqual(status["error"], "session-chat-negative-summary-failed")

            for mutation in ("boolean-schema", "duplicate-status", "duplicate-source"):
                malformed = self._rewrite_session_chat_archive(root, passed, mutation)
                result, status = self._run_session_chat_download(root, malformed)
                self.assertNotEqual(result.returncode, 0, mutation)
                self.assertEqual(status["sessionChatNegative"], "failed", mutation)

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
                "provision prepared service worker VM: Provider {",
                " provider: \"libkrun\",",
                " code: \"cgroup-place-worker\",",
                " source: Os { code: 13, kind: PermissionDenied, message: \"Permission denied\" },",
                "}",
                "thread 'cooking::smoke' panicked at crates/vm-libkrun/src/provider.rs:1057:9",
                "provision prepared service worker VM: unexpected libkrun provider error "
                "(runtime-permissions): Permission denied (os error 13)",
                "thread 'cooking::smoke' panicked at crates/vm-libkrun/src/provider.rs:1184:7",
                "provision prepared service worker VM: Unavailable { resource: \"worker spawn\", reason: \"Permission denied\" }",
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
                "  *' compute project-info describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"CPUS_ALL_REGIONS\",\"limit\":\"32\",\"usage\":\"0\"}]}' ;;\n"
                "  *' compute instances list '*) printf '[]\n' ;;\n"
                "  *' compute disks list '*) printf '[]\n' ;;\n"
                "  *' compute regions describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"SSD_TOTAL_GB\",\"limit\":\"1000\",\"usage\":\"0\"},{\"metric\":\"INSTANCES\",\"limit\":\"8\",\"usage\":\"0\"},{\"metric\":\"IN_USE_ADDRESSES\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
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
        self.assertIn(
            "HEPH_GCP_TEST test=rust-panic location=crates/foo/src/lib.rs:42:7 "
            "error_class=permission-denied errno=EACCES",
            result.stderr,
        )
        self.assertIn(
            "HEPH_GCP_TEST test=rust-panic operation=cgroup-place-worker "
            "error_class=permission-denied errno=EACCES",
            result.stderr,
        )
        self.assertIn(
            "HEPH_GCP_TEST test=rust-panic operation=runtime-permissions "
            "error_class=permission-denied errno=EACCES",
            result.stderr,
        )
        self.assertIn(
            "HEPH_GCP_TEST test=rust-panic location=crates/vm-libkrun/src/provider.rs:1184:7",
            result.stderr,
        )
        self.assertIn(
            "HEPH_GCP_TEST test=rust-panic operation=worker-spawn "
            "error_class=permission-denied errno=EACCES",
            result.stderr,
        )
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
                "  *' compute project-info describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"CPUS_ALL_REGIONS\",\"limit\":\"32\",\"usage\":\"0\"}]}' ;;\n"
                "  *' compute instances list '*) printf '[]\n' ;;\n"
                "  *' compute disks list '*) printf '[]\n' ;;\n"
                "  *' compute regions describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"SSD_TOTAL_GB\",\"limit\":\"1000\",\"usage\":\"0\"},{\"metric\":\"INSTANCES\",\"limit\":\"8\",\"usage\":\"0\"},{\"metric\":\"IN_USE_ADDRESSES\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
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
                "  *' compute project-info describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"CPUS_ALL_REGIONS\",\"limit\":\"32\",\"usage\":\"0\"}]}' ;;\n"
                "  *' compute instances list '*) printf '[]\n' ;;\n"
                "  *' compute disks list '*) printf '[]\n' ;;\n"
                "  *' compute regions describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"SSD_TOTAL_GB\",\"limit\":\"1000\",\"usage\":\"0\"},{\"metric\":\"INSTANCES\",\"limit\":\"8\",\"usage\":\"0\"},{\"metric\":\"IN_USE_ADDRESSES\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' storage objects describe '*) printf '%s\\n' '{\"name\":\"cooking/replacements/02430eca0a4e94ba129c4fdad969233f51486ee1dccdf1ee85e31f580f4d386d/heph-gcp-cooking-cache.tar.zst\",\"size\":\"1813939981\",\"md5Hash\":\"wt25yyIq4ikDQzFsaxFsdA==\",\"generation\":\"1\"}' ;;\n"
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
                "  *' compute project-info describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"CPUS_ALL_REGIONS\",\"limit\":\"32\",\"usage\":\"0\"}]}' ;;\n"
                "  *' compute instances list '*) printf '[]\n' ;;\n"
                "  *' compute disks list '*) printf '[]\n' ;;\n"
                "  *' compute regions describe '*) printf '%s\\n' "
                "'{\"quotas\":[{\"metric\":\"CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"SSD_TOTAL_GB\",\"limit\":\"1000\",\"usage\":\"0\"},{\"metric\":\"INSTANCES\",\"limit\":\"8\",\"usage\":\"0\"},{\"metric\":\"IN_USE_ADDRESSES\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' storage objects describe '*) printf '%s\\n' "
                "'{\"name\":\"cooking/replacements/02430eca0a4e94ba129c4fdad969233f51486ee1dccdf1ee85e31f580f4d386d/heph-gcp-cooking-cache.tar.zst\",\"size\":\"1813939981\",\"md5Hash\":\"wt25yyIq4ikDQzFsaxFsdA==\",\"generation\":\"1\"}' ;;\n"
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
                "  *' compute project-info describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"CPUS_ALL_REGIONS\",\"limit\":\"32\",\"usage\":\"0\"}]}' ;;\n"
                "  *' compute instances list '*) printf '[]\n' ;;\n"
                "  *' compute disks list '*) printf '[]\n' ;;\n"
                "  *' compute regions describe '*) printf '%s\\n' "
                "'{\"quotas\":[{\"metric\":\"CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"SSD_TOTAL_GB\",\"limit\":\"1000\",\"usage\":\"0\"},{\"metric\":\"INSTANCES\",\"limit\":\"8\",\"usage\":\"0\"},{\"metric\":\"IN_USE_ADDRESSES\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' storage objects describe '*) printf '%s\\n' "
                "'{\"name\":\"cooking/replacements/02430eca0a4e94ba129c4fdad969233f51486ee1dccdf1ee85e31f580f4d386d/heph-gcp-cooking-cache.tar.zst\",\"size\":\"1813939981\",\"md5Hash\":\"wt25yyIq4ikDQzFsaxFsdA==\",\"generation\":\"1\"}' ;;\n"
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
                "  *' compute project-info describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"CPUS_ALL_REGIONS\",\"limit\":\"32\",\"usage\":\"0\"}]}' ;;\n"
                "  *' compute instances list '*) printf '[]\n' ;;\n"
                "  *' compute disks list '*) printf '[]\n' ;;\n"
                "  *' compute regions describe '*) printf '%s\\n' "
                "'{\"quotas\":[{\"metric\":\"CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"SSD_TOTAL_GB\",\"limit\":\"1000\",\"usage\":\"0\"},{\"metric\":\"INSTANCES\",\"limit\":\"8\",\"usage\":\"0\"},{\"metric\":\"IN_USE_ADDRESSES\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' storage objects describe '*) printf '%s\\n' "
                "'{\"name\":\"cooking/replacements/02430eca0a4e94ba129c4fdad969233f51486ee1dccdf1ee85e31f580f4d386d/heph-gcp-cooking-cache.tar.zst\",\"size\":\"1813939981\",\"md5Hash\":\"wt25yyIq4ikDQzFsaxFsdA==\",\"generation\":\"1\"}' ;;\n"
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
                "  *' compute project-info describe '*) printf '%s\\n' '{\"quotas\":[{\"metric\":\"CPUS_ALL_REGIONS\",\"limit\":\"32\",\"usage\":\"0\"}]}' ;;\n"
                "  *' compute instances list '*) printf '[]\n' ;;\n"
                "  *' compute disks list '*) printf '[]\n' ;;\n"
                "  *' compute regions describe '*) printf '%s\\n' "
                "'{\"quotas\":[{\"metric\":\"CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"N2_CPUS\",\"limit\":\"32\",\"usage\":\"0\"},{\"metric\":\"SSD_TOTAL_GB\",\"limit\":\"1000\",\"usage\":\"0\"},{\"metric\":\"INSTANCES\",\"limit\":\"8\",\"usage\":\"0\"},{\"metric\":\"IN_USE_ADDRESSES\",\"limit\":\"8\",\"usage\":\"0\"}]}' ;;\n"
                "  *' storage objects describe '*) printf '%s\\n' "
                "'{\"name\":\"cooking/replacements/02430eca0a4e94ba129c4fdad969233f51486ee1dccdf1ee85e31f580f4d386d/heph-gcp-cooking-cache.tar.zst\",\"size\":\"1813939981\",\"md5Hash\":\"wt25yyIq4ikDQzFsaxFsdA==\",\"generation\":\"1\"}' ;;\n"
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
