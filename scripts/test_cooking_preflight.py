"""Regression coverage for typed Cooking preflight failure markers."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
PREFLIGHT = ROOT.parent / "examples" / "cooking" / "preflight.sh"
COLLECTOR_SPEC = importlib.util.spec_from_file_location(
    "cooking_diagnostics_collector", ROOT / "collect-cooking-diagnostics.py"
)
assert COLLECTOR_SPEC is not None and COLLECTOR_SPEC.loader is not None
COLLECTOR = importlib.util.module_from_spec(COLLECTOR_SPEC)
COLLECTOR_SPEC.loader.exec_module(COLLECTOR)


class CookingPreflightMarkerTests(unittest.TestCase):
    def run_preflight(
        self,
        *,
        missing_command: str | None = None,
        cgroup_valid: bool = True,
        podman_rootless: bool = True,
        python_probe: bool = True,
        rust_probe: bool = True,
    ) -> tuple[subprocess.CompletedProcess[str], str]:
        with tempfile.TemporaryDirectory(prefix="heph-cooking-preflight-") as raw:
            root = Path(raw)
            repo = root / "repo"
            cooking = repo / "examples" / "cooking"
            fake_bin = root / "bin"
            local_root = root / "local"
            builder_layout = root / "builder-layout"
            verifier_layout = root / "verifier-layout"
            base_manifest = root / "base-layout-manifest"
            cgroup = root / "cgroup"
            cooking.mkdir(parents=True)
            fake_bin.mkdir()
            local_root.mkdir()
            builder_layout.mkdir()
            verifier_layout.mkdir()
            base_manifest.touch()
            for layout in (builder_layout, verifier_layout):
                (layout / "index.json").touch()
                (layout / "oci-layout").touch()
            shutil.copy2(PREFLIGHT, cooking / "preflight.sh")
            (cooking / "preflight.sh").chmod(0o700)
            workflow = local_root / "repository-images"
            workflow.mkdir()
            (workflow / "workflow.env").write_text(
                "builder_vm_image=ubuntu@sha256:" + "a" * 64 + "\n"
                "verifier_vm_image=ubuntu@sha256:" + "b" * 64 + "\n"
                f"builder_layout={builder_layout}\n"
                f"verifier_layout={verifier_layout}\n"
                f"base_layout_manifest={base_manifest}\n",
                encoding="utf-8",
            )
            (cgroup / "cgroup.controllers").parent.mkdir()
            (cgroup / "cgroup.controllers").write_text("cpu io memory pids\n", encoding="utf-8")
            (cgroup / "cgroup.subtree_control").write_text("cpu io memory pids\n", encoding="utf-8")

            (fake_bin / "id").write_text(
                "#!/usr/bin/env bash\n"
                "case \"$1\" in -u|-g) printf '10001\\n' ;; *) /usr/bin/id \"$@\" ;; esac\n",
                encoding="utf-8",
            )
            (fake_bin / "uname").write_text(
                "#!/usr/bin/env bash\n"
                "case \"$1\" in -m) printf 'x86_64\\n' ;; *) /usr/bin/uname \"$@\" ;; esac\n",
                encoding="utf-8",
            )
            (fake_bin / "ldconfig").write_text(
                "#!/usr/bin/env bash\n"
                "printf 'libkrun.so.1 (libc6,x86-64) => /lib/libkrun.so.1\\n'\n"
                "printf 'libkrunfw.so.5 (libc6,x86-64) => /lib/libkrunfw.so.5\\n'\n",
                encoding="utf-8",
            )
            (fake_bin / "unshare").write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            podman_script = (
                "#!/usr/bin/env bash\n"
                "if [[ $1 == info ]]; then\n"
                f"  printf '%s\\n' {'true' if podman_rootless else 'false'}\n"
                "  exit 0\n"
                "fi\n"
                "if [[ $1 == image ]]; then exit 0; fi\n"
                "if [[ $1 == run ]]; then\n"
                "  if [[ $* == *'/usr/local/bin/python3'* ]]; then\n"
                f"    exit {0 if python_probe else 1}\n"
                "  fi\n"
                f"  exit {0 if rust_probe else 1}\n"
                "fi\n"
                "exit 1\n"
            )
            (fake_bin / "podman").write_text(podman_script, encoding="utf-8")
            for command in ("id", "uname", "ldconfig", "unshare", "podman"):
                (fake_bin / command).chmod(0o700)
            for command in ("cargo", "curl", "debugfs", "musl-gcc", "rustup"):
                path = fake_bin / command
                path.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
                path.chmod(0o700)

            environment = {
                **os.environ,
                "PATH": f"{fake_bin}:/usr/bin:/bin",
                "HEPHAESTUS_LOCAL_ROOT": str(local_root),
                "HEPHAESTUS_LIBKRUN_CGROUP_PARENT": str(cgroup if cgroup_valid else root / "missing-cgroup"),
                "HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE": "ubuntu@sha256:" + "c" * 64,
                "HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE": "ubuntu@sha256:" + "d" * 64,
            }
            if missing_command is not None:
                # The command is selected from the fixed preflight list; its
                # absence is supplied by omitting it from PATH and fake_bin.
                environment["PATH"] = f"{fake_bin}:/usr/bin:/bin"
                (fake_bin / missing_command).unlink(missing_ok=True)
            result = subprocess.run(
                [str(cooking / "preflight.sh")],
                cwd=repo,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )
            source = root / "preflight-stderr.log"
            source.write_text(result.stderr, encoding="utf-8")
            projected = root / "projected.log"
            COLLECTOR._project_text(source, projected)
            return result, projected.read_text(encoding="utf-8")

    def test_missing_command_marker_identifies_fixed_tool(self) -> None:
        result, projected = self.run_preflight(missing_command="cargo")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "stage=preflight-command-checks reason_class=missing-command status=failed exit_code=1 test=cargo",
            projected,
        )

    def test_cgroup_failure_marker_identifies_delegation(self) -> None:
        result, projected = self.run_preflight(cgroup_valid=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stage=cgroup-delegation reason_class=delegation-unavailable", projected)

    def test_rootful_podman_marker_identifies_sandbox_requirement(self) -> None:
        result, projected = self.run_preflight(podman_rootless=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stage=podman-rootless reason_class=rootful-podman", projected)

    def test_python_probe_marker_identifies_guest_probe(self) -> None:
        result, projected = self.run_preflight(python_probe=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stage=python-image-probe reason_class=image-probe-failed status=failed", projected)
        self.assertIn("test=python-image", projected)

    def test_rust_probe_marker_identifies_guest_probe(self) -> None:
        result, projected = self.run_preflight(rust_probe=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stage=rust-image-probe reason_class=image-probe-failed status=failed", projected)
        self.assertIn("test=rust-image", projected)


if __name__ == "__main__":
    unittest.main()
