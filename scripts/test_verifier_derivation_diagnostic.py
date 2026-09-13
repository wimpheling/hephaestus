"""Real isolated-entrypoint/collector checks; opt in with reviewed local inputs."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tarfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


@unittest.skipUnless(os.environ.get("HEPHAESTUS_TEST_DIAGNOSTIC_LOCAL_ROOT"), "requires reviewed baseline workflow and imported Podman image")
class DiagnosticLifecycle(unittest.TestCase):
    def exercise(self, mode, expected, label):
        with tempfile.TemporaryDirectory(prefix="heph-diagnostic-check-") as temporary:
            root = Path(temporary)
            env = dict(os.environ, TMPDIR=str(root), HEPHAESTUS_LOCAL_ROOT=os.environ["HEPHAESTUS_TEST_DIAGNOSTIC_LOCAL_ROOT"])
            if mode == "unknown":
                env["HEPHAESTUS_LOCAL_ROOT"] = str(root / "missing-workflow")
            if mode == "command":
                wrapper = root / "bin"
                wrapper.mkdir()
                podman = wrapper / "podman"
                podman.write_text("#!/usr/bin/env python3\nimport os,sys\na=sys.argv[1:]\nif '/usr/bin/umoci' in a:\n i=a.index('--entrypoint'); a=a[:i]+['--entrypoint','/bin/false',a[i+2]]\nos.execv(" + repr(shutil.which("podman")) + ", ['podman']+a)\n")
                podman.chmod(0o755)
                env["PATH"] = str(wrapper) + os.pathsep + env["PATH"]
            if mode in ("inspect-command-error", "inspect-digest-mismatch"):
                wrappers = root / "bin"
                wrappers.mkdir()
                workflow = dict(line.split("=", 1) for line in (Path(env["HEPHAESTUS_LOCAL_ROOT"]) / "repository-images/workflow.env").read_text().splitlines() if "=" in line)
                alternative = "containers-storage:" + workflow["builder_vm_image"]
                real = shutil.which("skopeo")
                code = "#!/usr/bin/env python3\nimport os,sys\na=sys.argv[1:]\n"
                code += "if os.geteuid()!=0 and a[0]=='inspect':\n a[-1]=" + repr("containers-storage:localhost/heph-nonexistent-inspection-fixture" if mode == "inspect-command-error" else alternative) + "\n"
                code += "if os.geteuid()!=0 and a[0]=='copy':\n a[-2]='oci:/nonexistent-private-fixture-source'\n"
                code += "os.execv(" + repr(real) + ", ['skopeo']+a)\n"
                wrapper = wrappers / "skopeo"
                wrapper.write_text(code)
                wrapper.chmod(0o755)
                env["PATH"] = str(wrappers) + os.pathsep + env["PATH"]
            result = subprocess.run([str(ROOT / "examples/cooking/run.sh")], env=env, capture_output=True, text=True, timeout=100)
            self.assertEqual(result.returncode, expected, result.stderr)
            markers = [line for line in result.stderr.splitlines() if line.startswith("HEPH_GCP_SHELL_FAILURE ")]
            expected_codes = {"success": [78, 80], "command": [41], "unknown": [79], "inspect-command-error": [49, 78, 81, 82], "inspect-digest-mismatch": [53, 78, 81, 82]}[mode]
            self.assertEqual([int(dict(word.split("=", 1) for word in marker.split()[1:])["exit_code"]) for marker in markers], expected_codes, result.stderr)
            fields = dict(word.split("=", 1) for word in markers[0].split()[1:])
            self.assertEqual(int(fields["exit_code"]), expected)
            source_line = (ROOT / "scripts/run-libkrun-integration.sh").read_text().splitlines()[int(fields["line"]) - 1]
            self.assertIn(label, source_line)
            self.assertFalse(list(root.glob("verifier-diagnostic.*")), "private output cleanup")
            serial = root / "serial"
            serial.write_text(result.stderr)
            gates = root / "gates.json"
            revision = subprocess.check_output(["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip()
            gates.write_text(json.dumps({"schema":1,"revision":revision,"script_sha256":"a"*64,"test_mode":"gcp-cooking","overall_exit_code":expected,"supervisor_exit_code":expected,"finalized":True,"gates":{"workload":{"state":"failed","exit_code":expected,"reason_class":"workload-failed"},"evidence-scan":{"state":"passed","exit_code":0,"reason_class":"none"},"browser-validation":{"state":"failed","exit_code":2,"reason_class":"browser-report-invalid"}}}))
            bundle = root / "bundle"
            collected = subprocess.run(["python3", str(ROOT / "scripts/collect-cooking-diagnostics.py"), "--output-dir", str(bundle), "--source", "serial="+str(serial), "--source", "gate-results="+str(gates)], capture_output=True, text=True)
            self.assertEqual(collected.returncode, 0, collected.stderr)
            archive = root / "diagnostics.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                for member in bundle.rglob("*"):
                    if member.is_file():
                        output.add(member, arcname="cooking-diagnostics/" + str(member.relative_to(bundle)), recursive=False)
            validated = subprocess.run(["bash", str(ROOT / "scripts/export-encrypted-diagnostics.sh"), "validate", str(archive)], capture_output=True, text=True)
            self.assertEqual(validated.returncode, 0, validated.stderr)
            summary = subprocess.run(["python3", str(ROOT / "scripts/summarize-cooking-diagnostics.py"), str(bundle)], capture_output=True, text=True)
            self.assertEqual(summary.returncode, 0, summary.stderr)
            value = json.loads(summary.stdout)
            matched = [entry for entry in value["technicalContext"] if entry.get("kind") == "shell-failure"]
            self.assertEqual(len(matched), len(expected_codes))
            self.assertEqual([item["exit_code"] for item in matched], expected_codes)
            self.assertEqual(matched[0]["exit_code"], expected)
            self.assertEqual(str(matched[0]["line"]), fields["line"])
            self.assertNotIn("Traceback", summary.stdout)
            self.assertNotIn(str(root), summary.stdout)
            self.assertEqual(json.loads((bundle / "sources/gate-results").read_text())["revision"], revision)

    def test_real_helper_success_is_intentional_failure(self):
        self.exercise("success", 78, "helper-passed-intentional-stop")

    def test_real_container_failure_identifies_add_layer(self):
        self.exercise("command", 41, "podman-umoci-add-layer")

    def test_real_inspect_command_failure_and_namespace_remedy(self):
        self.exercise("inspect-command-error", 49, "direct-command-error-unshare-wrapper-match")

    def test_real_wrong_digest_and_namespace_remedy(self):
        self.exercise("inspect-digest-mismatch", 53, "direct-digest-mismatch-unshare-wrapper-match")

    def test_unknown_failure_is_closed(self):
        self.exercise("unknown", 79, "unknown-fail-closed")


if __name__ == "__main__":
    unittest.main()
