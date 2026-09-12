"""Executable boundary contracts for the GCP PR Cooking workload sandbox."""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
RUNTIME = ROOT / "gcp-cooking-run.sh"


class GcpPrSandboxTests(unittest.TestCase):
    def test_trusted_image_import_uses_the_pr_store_namespace(self) -> None:
        """The trusted import and PR workload must share private Podman state."""

        source = RUNTIME.read_text(encoding="utf-8")
        configure = source.index(
            'if [[ "$workload_trust" == untrusted-pr ]]; then\n'
            '    # Configure the private HOME/runtime before trusted image import'
        )
        workflow_phase = source.index("phase_start workflow-images")
        self.assertLess(configure, workflow_phase)
        workflow_start = source.index(
            'run_with_deadline systemd-run --unit="heph-gcp-cooking-images-'
        )
        workflow_end = source.index("\nphase_pass", workflow_start)
        workflow = source[workflow_start:workflow_end]
        self.assertIn('--setenv=HOME="$workload_home"', workflow)
        self.assertIn('--setenv=XDG_DATA_HOME="$workload_home/.local/share"', workflow)
        self.assertIn('"${workflow_image_sandbox_args[@]}"', workflow)
        self.assertIn(
            '"--property=BindPaths=$pr_runtime:/run/user/10001"',
            source,
        )
        self.assertIn(
            'workflow_image_sandbox_args=("${pr_sandbox_filesystem_args[@]}")',
            source,
        )

    def test_root_helpers_ignore_forge_writable_cargo_bin(self) -> None:
        """A forge-planted interpreter must not affect root-side helpers."""

        source = RUNTIME.read_text(encoding="utf-8")
        self.assertIn("readonly trusted_path=", source)
        self.assertIn('PATH="$trusted_path"', source)
        self.assertIn('workload_path="/home/forge/.cargo/bin:', source)
        prefix = source[: source.index("\nreadonly metadata_root=")]
        with tempfile.TemporaryDirectory(prefix="heph-root-path-test-") as directory:
            root = Path(directory)
            cargo_bin = root / "cargo-bin"
            cargo_bin.mkdir()
            planted = cargo_bin / "python3"
            planted.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'forge-planted-interpreter\n'\n"
                "exit 77\n",
                encoding="utf-8",
            )
            planted.chmod(0o700)
            probe = prefix + "\ncommand -v python3\npython3 -c 'print(\"trusted-interpreter\")'\n"
            result = subprocess.run(
                ["/bin/bash", "-Eeuo", "pipefail", "-c", probe],
                env={**os.environ, "PATH": str(cargo_bin)},
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn("forge-planted-interpreter", result.stdout)
            self.assertIn("trusted-interpreter", result.stdout)

    def test_read_write_and_inaccessible_paths_are_single_systemd_arguments(self) -> None:
        """Each systemd property must receive its complete path list as one argv."""

        source = RUNTIME.read_text(encoding="utf-8")
        start = source.index("    pr_sandbox_filesystem_args=(\n") + len("    ")
        end = source.index("\n    )", start) + len("\n    )")
        assignment = source[start:end]
        result = subprocess.run(
            ["bash", "-Eeuo", "pipefail", "-c", f"""
                checkout_root=/srv/hephaestus/checkout
                cache_root=/srv/hephaestus/cooking-cache
                evidence_root=/srv/hephaestus/evidence/cooking
                smoke_temporary_root=/tmp/hephaestus-libkrun
                browser_root=/srv/hephaestus/playwright-browsers
                pr_state_root=/srv/hephaestus/pr-state
                pr_runtime=/srv/hephaestus/pr-state/runtime
                pr_tmp_root=/srv/hephaestus/pr-state/tmp
                pr_var_tmp_root=/srv/hephaestus/pr-state/var-tmp
                {assignment}
                pr_sandbox_args=( '--property=NoNewPrivileges=yes' "${{pr_sandbox_filesystem_args[@]}}" )
                printf '<%s>\\n' "${{pr_sandbox_args[@]}}"
            """],
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        args = [line[1:-1] for line in result.stdout.splitlines()]
        self.assertTrue(args)
        self.assertTrue(all(arg.startswith("--property=") for arg in args), args)
        properties: dict[str, list[str]] = {}
        for arg in args:
            key, value = arg.split("=", 2)[1:]
            properties.setdefault(key, []).extend(value.split())
        self.assertEqual(
            set(properties["ReadWritePaths"]),
            {
                "/srv/hephaestus/checkout",
                "/srv/hephaestus/evidence/cooking",
                "/srv/hephaestus/pr-state",
            },
        )
        self.assertEqual(
            set(properties["BindReadOnlyPaths"]),
            {
                "/srv/hephaestus/cooking-cache",
                "/srv/hephaestus/playwright-browsers",
                "/home/forge/.rustup",
            },
        )
        self.assertEqual(
            set(properties["BindPaths"]),
            {
                "/home/forge/.cargo",
                "/srv/hephaestus/pr-state",
                "/srv/hephaestus/pr-state/runtime:/run/user/10001",
                "/srv/hephaestus/pr-state/tmp:/tmp",
                "/srv/hephaestus/pr-state/var-tmp:/var/tmp",
            },
        )
        self.assertEqual(
            set(properties["InaccessiblePaths"]),
            {"/var/log/hephaestus", "/run/hephaestus", "/root"},
        )

    def test_pr_setup_freezes_cache_and_enters_sandbox_before_npm(self) -> None:
        """PR lifecycle code must run after cache freeze inside a restricted unit."""

        source = RUNTIME.read_text(encoding="utf-8")
        start = source.index("# The cache is a trusted immutable input")
        end = source.index("\nphase_start cooking", start)
        setup_flow = source[start:end]
        function_start = source.index("configure_pr_sandbox()")
        function_end = source.index("\n}\n", function_start) + len("\n}")
        sandbox_function = source[function_start:function_end]
        with tempfile.TemporaryDirectory(prefix="heph-pr-sandbox-test-") as directory:
            root = Path(directory)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            checkout = root / "checkout"
            playwright = checkout / "e2e" / "playwright"
            playwright.mkdir(parents=True)
            (playwright / "package-lock.json").write_text("{}\n", encoding="utf-8")
            browser_root = root / "browsers"
            browser_root.mkdir()
            browser = browser_root / "chrome-headless-shell"
            browser.write_text(
                "#!/usr/bin/env bash\nprintf 'Chromium 1.2.3\\n'\n", encoding="utf-8"
            )
            browser.chmod(0o700)
            cache_root = root / "cache"
            cache_root.mkdir()
            log = root / "events.log"
            nft_state = root / "nft.state"

            self._fake(
                fake_bin / "install",
                """#!/usr/bin/env bash
set -Eeuo pipefail
for arg in "$@"; do
  [[ "$arg" == /* ]] && mkdir -p "$arg"
done
""",
            )
            self._fake(
                fake_bin / "runuser",
                """#!/usr/bin/env bash
set -Eeuo pipefail
printf 'runuser %q ' "$@" >>"$FAKE_LOG"
printf '\\n' >>"$FAKE_LOG"
while (($#)); do
  [[ "$1" == -- ]] && { shift; break; }
  shift
done
export FAKE_FORGE=1
exec "$@"
""",
            )
            self._fake(
                fake_bin / "systemd-run",
                """#!/usr/bin/env bash
set -Eeuo pipefail
printf 'systemd-run %q ' "$@" >>"$FAKE_LOG"
printf '\\n' >>"$FAKE_LOG"
command_index=0
for arg in "$@"; do
  if [[ "$arg" == -- ]]; then
    command_index=$((command_index + 1))
    break
  fi
  [[ "$arg" == --* ]] || break
  command_index=$((command_index + 1))
done
export FAKE_FORGE=1
exec "${@:$((command_index + 1))}"
""",
            )
            self._fake(
                fake_bin / "npm",
                """#!/usr/bin/env bash
set -Eeuo pipefail
printf 'npm FAKE_FORGE=%s\\n' "${FAKE_FORGE:-missing}" >>"$FAKE_LOG"
""",
            )
            self._fake(
                fake_bin / "find",
                """#!/usr/bin/env bash
set -Eeuo pipefail
if printf '%s\\n' "$@" | grep -qx -- '-exec'; then
  printf 'cache-freeze\\n' >>"$FAKE_LOG"
else
  printf '%s\\n' "$BROWSER_PATH"
fi
""",
            )
            self._fake(
                fake_bin / "nft",
                f"""#!/usr/bin/env bash
set -Eeuo pipefail
if [[ "$1" == list ]]; then
  [[ -e {nft_state!s} ]] || exit 1
  printf '169.254.169.254 fd20:ce::254\\n'
  exit 0
fi
touch {nft_state!s}
""",
            )
            self._fake(
                fake_bin / "curl",
                """#!/usr/bin/env bash
set -Eeuo pipefail
[[ "${FAKE_FORGE:-0}" == 1 ]] && exit 1
exit 0
""",
            )

            lock_sha = hashlib.sha256((playwright / "package-lock.json").read_bytes()).hexdigest()
            harness = f"""
set -Eeuo pipefail
checkout_root={self._shell_quote(checkout)}
browser_root={self._shell_quote(browser_root)}
cache_root={self._shell_quote(cache_root)}
evidence_root={self._shell_quote(root / 'evidence')}
smoke_temporary_root={self._shell_quote(root / 'tmp')}
work_root={self._shell_quote(root)}
pr_state_root={self._shell_quote(root / 'pr-state')}
pr_runtime={self._shell_quote(root / 'pr-state' / 'runtime')}
pr_tmp_root={self._shell_quote(root / 'pr-state' / 'tmp')}
pr_var_tmp_root={self._shell_quote(root / 'pr-state' / 'var-tmp')}
pr_home={self._shell_quote(root / 'pr-state' / 'home')}
pr_cargo_home={self._shell_quote(root / 'pr-state' / 'cargo')}
pr_rustup_home={self._shell_quote(root / 'pr-state' / 'rustup')}
pr_npm_cache={self._shell_quote(root / 'pr-state' / 'npm-cache')}
pr_sandbox_args=()
workload_home=/home/forge
workload_path=/home/forge/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
workload_cargo_home=/home/forge/.cargo
workload_rustup_home=/home/forge/.rustup
workload_npm_cache=''
metadata_guard_table=hephaestus_gcp_metadata_guard
metadata_ip=169.254.169.254
metadata_ipv6=fd20:ce::254
forge_uid=10001
forge_gid=10001
runner_image_verified=true
runner_image_browser_lock_sha={lock_sha}
runner_image_browser_version='Chromium 1.2.3'
workload_trust=untrusted-pr
phase_start() {{ :; }}
phase_pass() {{ :; }}
require_command() {{ command -v "$1" >/dev/null; }}
run_with_deadline() {{ "$@"; }}
fail() {{ printf 'failure: %s\\n' "$*" >&2; exit 1; }}
export FAKE_LOG={self._shell_quote(log)} BROWSER_PATH={self._shell_quote(browser)}
{sandbox_function}
configure_pr_sandbox
{setup_flow}
"""
            result = subprocess.run(
                ["bash", "-Eeuo", "pipefail", "-c", harness],
                env={**os.environ, "PATH": f"{fake_bin}:/usr/bin:/bin"},
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            events = log.read_text(encoding="utf-8").splitlines()
            npm_index = next(i for i, event in enumerate(events) if event.startswith("npm "))
            freeze_indices = [i for i, event in enumerate(events) if event == "cache-freeze"]
            self.assertTrue(freeze_indices, events)
            self.assertLess(max(freeze_indices), npm_index, events)
            systemd_indices = [i for i, event in enumerate(events) if event.startswith("systemd-run ")]
            self.assertTrue(systemd_indices, events)
            self.assertLess(max(i for i in systemd_indices if i < npm_index), npm_index, events)
            systemd = "\n".join(events[i] for i in systemd_indices)
            for required in (
                "NoNewPrivileges=yes",
                "ProtectProc=invisible",
                "ProcSubset=pid",
                "ProtectSystem=strict",
                "ProtectHome=tmpfs",
                "PrivateTmp=yes",
            ):
                self.assertIn(required, systemd)
            self.assertIn("npm FAKE_FORGE=1", events[npm_index])

    @staticmethod
    def _fake(path: Path, content: str) -> None:
        path.write_text(content, encoding="utf-8")
        path.chmod(0o700)

    @staticmethod
    def _shell_quote(path: Path) -> str:
        import shlex

        return shlex.quote(str(path))


if __name__ == "__main__":
    unittest.main()
