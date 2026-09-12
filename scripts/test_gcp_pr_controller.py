"""Static contracts for the trusted main-controller PR Cooking path."""

from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
WORKFLOW = ROOT.parent / ".github" / "workflows" / "cooking-e2e.yml"
COORDINATOR = ROOT / "gcp-kvm-smoke.sh"
STARTUP = ROOT / "gcp-kvm-startup.sh"
RUNTIME = ROOT / "gcp-cooking-run.sh"


class PrControllerContractTests(unittest.TestCase):
    def test_workflow_keeps_main_controller_and_declares_bounded_pr_inputs(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("pr_number:", workflow)
        self.assertIn("pr_repository_id:", workflow)
        self.assertIn("pr_head_sha:", workflow)
        self.assertIn("GCP_PR_NUMBER: ${{ inputs.pr_number }}", workflow)
        self.assertIn("GH_TOKEN: ${{ github.token }}", workflow)
        self.assertIn("github.ref == 'refs/heads/main'", workflow)
        self.assertNotIn("pull_request_target", workflow)

    def test_controller_validates_provenance_before_create_and_anchors_labels(self) -> None:
        coordinator = COORDINATOR.read_text(encoding="utf-8")
        self.assertIn("validate_workload_provenance", coordinator)
        self.assertIn("PR provenance inputs must be supplied together", coordinator)
        self.assertIn("verify-gcp-pr-provenance.py", coordinator)
        self.assertIn("github-sha=$validated_source_sha", coordinator)
        self.assertIn("sha=$validated_source_sha", coordinator)
        self.assertLess(
            coordinator.index("validate_workload_provenance"),
            coordinator.index("gcloud compute instances create"),
        )

    def test_root_helpers_are_staged_from_trusted_metadata(self) -> None:
        startup = STARTUP.read_text(encoding="utf-8")
        runtime = RUNTIME.read_text(encoding="utf-8")
        self.assertIn("trusted_cooking_runtime_script", startup)
        self.assertIn("cooking-runtime-script-sha256", startup)
        self.assertIn('"$trusted_cooking_runtime_script"', startup)
        self.assertNotIn('"$checkout_root/scripts/gcp-cooking-run.sh"', startup)
        self.assertIn("HEPH_GCP_COOKING_GATE_RESULTS_HELPER", startup)
        self.assertIn("HEPH_GCP_DIAGNOSTICS_SCANNER_SCRIPT", startup)
        self.assertIn("HEPH_GCP_BROWSER_SUMMARY_SCRIPT", startup)
        self.assertIn("phase-timing-script", startup)
        self.assertIn("trusted_phase_timing_script", startup)
        self.assertIn("HEPH_GCP_COOKING_GATE_RESULTS_HELPER:-", runtime)
        self.assertIn("HEPH_GCP_DIAGNOSTICS_SCANNER_SCRIPT:-", runtime)
        self.assertIn("HEPH_GCP_BROWSER_SUMMARY_SCRIPT:-", runtime)
        self.assertIn("HEPH_GCP_PHASE_TIMING_PATH", runtime)

    def test_root_collection_does_not_fallback_to_pr_helpers(self) -> None:
        startup = STARTUP.read_text(encoding="utf-8")
        collection = startup[startup.index("collect_diagnostics()") :]
        self.assertIn('collector="$diagnostics_metadata_root/collect-cooking-diagnostics.py"', collection)
        self.assertIn('scanner="$diagnostics_metadata_root/check-browser-evidence.py"', collection)
        self.assertNotIn('collector="$checkout_root/scripts/collect-cooking-diagnostics.py"', collection)
        self.assertNotIn('scanner="$checkout_root/scripts/check-browser-evidence.py"', collection)

    def test_pr_sandbox_properties_are_single_argv_values(self) -> None:
        runtime = RUNTIME.read_text(encoding="utf-8")
        start = runtime.index("configure_pr_sandbox() {")
        end = runtime.index("\n}\n", start) + 2
        function = runtime[start:end]
        with tempfile.TemporaryDirectory(prefix="heph-pr-sandbox-") as directory:
            root = Path(directory)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            fake_install = fake_bin / "install"
            fake_install.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "args=()\n"
                "while (($#)); do\n"
                "  case $1 in -o|-g) shift 2 ;; *) args+=(\"$1\"); shift ;; esac\n"
                "done\n"
                "/usr/bin/install \"${args[@]}\"\n",
                encoding="utf-8",
            )
            fake_install.chmod(0o700)
            command = f'''#!/usr/bin/env bash
set -Eeuo pipefail
export PATH={fake_bin}:$PATH
work_root={root}
checkout_root={root}/checkout
evidence_root={root}/evidence
browser_root={root}/browsers
smoke_temporary_root={root}/tmp
cache_root={root}/cache
workload_trust=untrusted-pr
pr_state_root={root}/pr-state
pr_runtime={root}/pr-state/runtime
pr_home={root}/pr-state/home
pr_cargo_home={root}/pr-state/cargo
pr_rustup_home={root}/pr-state/rustup
pr_npm_cache={root}/pr-state/npm-cache
{function}
configure_pr_sandbox
printf '<%s>\\n' "${{pr_sandbox_args[@]}}"
'''
            result = subprocess.run(
                ["bash", "-Eeuo", "pipefail", "-c", command],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            args = [line[1:-1] for line in result.stdout.splitlines()]
            self.assertIn("--property=ReadWritePaths=" + " ".join(
                (
                    str(root / "checkout"),
                    str(root / "evidence"),
                    str(root / "pr-state"),
                )
            ), args)
            self.assertIn("--property=InaccessiblePaths=/var/log/hephaestus /run/hephaestus /root", args)
            self.assertIn(f"--property=BindReadOnlyPaths={root / 'browsers'}", args)
            self.assertIn("--property=BindPaths=/home/forge/.cargo", args)
            self.assertIn(
                f"--property=BindPaths={root / 'pr-state' / 'runtime'}:/run/user/10001",
                args,
            )
            self.assertNotIn(str(root / "evidence"), args)
            self.assertNotIn(str(root / "tmp"), args)

    def test_pr_browser_setup_and_workload_share_the_sandbox(self) -> None:
        runtime = RUNTIME.read_text(encoding="utf-8")
        browser_setup_start = runtime.index(
            'if [[ "$workload_trust" == untrusted-pr ]]; then',
            runtime.index("playwright_npm_env=()"),
        )
        setup = runtime[browser_setup_start:]
        self.assertIn('systemd-run \\\n        --unit="heph-gcp-pr-browser-setup-', setup)
        self.assertIn('"${pr_sandbox_args[@]}" "${pr_playwright_npm_args[@]}"', setup)
        workload = runtime[runtime.index("run_cooking_workload() {"):]
        self.assertIn('"${pr_sandbox_args[@]}" \\\n', workload)
        self.assertIn('--property=KillMode=control-group', setup)
        self.assertNotIn('runuser -u forge -- env HOME=/home/forge', setup[: setup.index('else')])

    def test_pr_workload_invocation_passes_sandbox_as_complete_argv(self) -> None:
        runtime = RUNTIME.read_text(encoding="utf-8")
        start = runtime.index("run_cooking_workload() {")
        end = runtime.index("\n}\n", start) + 2
        function = runtime[start:end]
        with tempfile.TemporaryDirectory(prefix="heph-pr-workload-") as directory:
            root = Path(directory)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            fake_install = fake_bin / "install"
            fake_install.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "args=()\n"
                "while (($#)); do\n"
                "  case $1 in -o|-g) shift 2 ;; *) args+=(\"$1\"); shift ;; esac\n"
                "done\n"
                "/usr/bin/install \"${args[@]}\"\n",
                encoding="utf-8",
            )
            fake_install.chmod(0o700)
            fake_systemd = fake_bin / "systemd-run"
            fake_systemd.write_text(
                "#!/usr/bin/env bash\n"
                "set -Eeuo pipefail\n"
                "printf '%s\\n' \"$@\" >\"$ARGS_FILE\"\n",
                encoding="utf-8",
            )
            fake_systemd.chmod(0o700)
            args_file = root / "systemd-run.args"
            checkout = root / "checkout"
            evidence = root / "evidence"
            checkout.mkdir()
            evidence.mkdir()
            command = f'''#!/usr/bin/env bash
set -Eeuo pipefail
export PATH={fake_bin}:$PATH
export ARGS_FILE={args_file}
workload_started=true
cooking_remaining=5
cooking_timeout=4
cooking_unit=heph-test-cooking
forge_uid=10001
forge_gid=10001
checkout_root={checkout}
evidence_root={evidence}
browser_root={root}/browsers
cache_root={root}/cache
runtime_python_ref=python-ref
runtime_rust_ref=rust-ref
workload_trust=untrusted-pr
workload_home={root}/pr-state/home
workload_path=/home/forge/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
workload_cargo_home=/home/forge/.cargo
workload_rustup_home=/home/forge/.rustup
pr_sandbox_args=(
  '--property=ReadWritePaths={checkout} {evidence} {root}/pr-state'
  '--property=BindPaths={root}/pr-state/runtime:/run/user/10001'
  '--property=InaccessiblePaths=/var/log/hephaestus /run/hephaestus /root'
)
{function}
run_cooking_workload
'''
            result = subprocess.run(
                ["bash", "-Eeuo", "pipefail", "-c", command],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            args = args_file.read_text(encoding="utf-8").splitlines()
            self.assertIn(
                f"--property=ReadWritePaths={checkout} {evidence} {root / 'pr-state'}",
                args,
            )
            self.assertIn(
                f"--property=BindPaths={root / 'pr-state' / 'runtime'}:/run/user/10001",
                args,
            )
            self.assertIn("--property=InaccessiblePaths=/var/log/hephaestus /run/hephaestus /root", args)
            self.assertIn(f"--setenv=HOME={root / 'pr-state' / 'home'}", args)
            self.assertNotIn(str(evidence), args)
            self.assertNotIn("runuser", args)


if __name__ == "__main__":
    unittest.main()
