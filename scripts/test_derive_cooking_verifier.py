#!/usr/bin/env python3
"""Real pinned Podman/Umoci fixtures, explicitly enabled with a local OCI layout.

HEPHAESTUS_TEST_VERIFIER_LAYOUT=/absolute/reviewed/layout python3 -m unittest discover -s scripts -p test_derive_cooking_verifier.py
No registry pull or VM is used.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import uuid


@unittest.skipUnless(os.environ.get("HEPHAESTUS_TEST_VERIFIER_LAYOUT"), "requires reviewed local verifier layout and rootless Podman")
class DerivationLifecycle(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.reviewed_layout = Path(os.environ["HEPHAESTUS_TEST_VERIFIER_LAYOUT"])
        digest = json.loads((cls.reviewed_layout / "index.json").read_text())["manifests"][0]["digest"]
        cls.reviewed_reference = "localhost:55000/platform/images/oci-verifier-ubuntu@" + digest
        status = subprocess.run(["podman", "image", "exists", cls.reviewed_reference], check=False).returncode
        if status not in (0, 1):
            raise RuntimeError("rootless Podman is unavailable")
        cls.owned_baseline = status == 1
        if cls.owned_baseline:
            subprocess.run(["skopeo", "copy", "--preserve-digests", "oci:" + str(cls.reviewed_layout), "containers-storage:" + cls.reviewed_reference], check=True, stdout=subprocess.DEVNULL)
            cls.addClassCleanup(subprocess.run, ["podman", "rmi", cls.reviewed_reference], check=True, stdout=subprocess.DEVNULL)

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="heph-verifier-cli-")
        self.root = Path(self.directory.name)
        self.layout = Path(os.environ["HEPHAESTUS_TEST_VERIFIER_LAYOUT"])
        digest = json.loads((self.layout / "index.json").read_text())["manifests"][0]["digest"]
        self.reference = "localhost:55000/platform/images/oci-verifier-ubuntu@" + digest
        self.helper = Path(__file__).with_name("derive-cooking-verifier.py")
        self.script = self.root / "oci-verify"
        self.script.write_text("#!/bin/sh\nprintf 'private fixture\\n'\n")
        self.revision = "1" * 40

    def tearDown(self):
        self.directory.cleanup()

    def run_helper(self, name, env=None, reference=None):
        output = self.root / name
        result = subprocess.run(["python3", str(self.helper), "--baseline-reference", reference or self.reference, "--baseline-layout", str(self.layout), "--script", str(self.script), "--source-revision", self.revision, "--output", str(output)], capture_output=True, text=True, env=env)
        return result, output

    def test_derive_deterministic_and_real_mapping(self):
        first, output = self.run_helper("first")
        self.assertEqual(first.returncode, 0, first.stderr)
        record = json.loads((output / "derivation.json").read_text())
        second, other = self.run_helper("second")
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(record["reference"], json.loads((other / "derivation.json").read_text())["reference"])
        self.assertTrue(record["derived"])
        self.assertFalse((output / "container-id").exists())
        # Import and execute only fixed inspection commands, never the inserted
        # script: this proves actual layer ownership/mode through Podman.
        subprocess.run(["skopeo", "copy", "--preserve-digests", "oci:" + record["layout"], "containers-storage:" + record["reference"]], check=True, stdout=subprocess.DEVNULL)
        try:
            actual = subprocess.check_output(["podman", "run", "--rm", "--pull", "never", "--network", "none", "--entrypoint", "/bin/sh", record["reference"], "-ec", "stat -c '%u:%g:%a' /usr/libexec/hephaestus/oci-verify; sha256sum /usr/libexec/hephaestus/oci-verify"], text=True).splitlines()
            self.assertEqual(actual[0], "0:0:555")
            self.assertEqual(actual[1].split()[0], hashlib.sha256(self.script.read_bytes()).hexdigest())
        finally:
            subprocess.run(["podman", "rmi", record["reference"]], check=True, stdout=subprocess.DEVNULL)

    def test_reuse_original(self):
        self.script.write_bytes(subprocess.check_output(["podman", "run", "--rm", "--pull", "never", "--network", "none", "--entrypoint", "/bin/cat", self.reference, "/usr/libexec/hephaestus/oci-verify"]))
        result, output = self.run_helper("reuse")
        self.assertEqual(result.returncode, 0, result.stderr)
        record = json.loads((output / "derivation.json").read_text())
        self.assertFalse(record["derived"])
        self.assertEqual(record["reference"], self.reference)
        self.assertFalse((output / "image").exists())

    def test_owned_import_is_removed(self):
        alias = "localhost/heph-verifier-fixture-" + uuid.uuid4().hex + "@" + self.reference.split("@", 1)[1]
        self.assertEqual(subprocess.run(["podman", "image", "exists", alias], check=False).returncode, 1)
        self.script.write_bytes(subprocess.check_output(["podman", "run", "--rm", "--pull", "never", "--network", "none", "--entrypoint", "/bin/cat", self.reference, "/usr/libexec/hephaestus/oci-verify"]))
        try:
            result, output = self.run_helper("owned-import", reference=alias)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads((output / "derivation.json").read_text())["derived"])
            self.assertEqual(subprocess.run(["podman", "image", "exists", alias], check=False).returncode, 1)
            self.assertEqual(subprocess.run(["podman", "image", "exists", self.reference], check=False).returncode, 0)
        finally:
            subprocess.run(["podman", "rmi", alias], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def test_reject_digest_and_symlink(self):
        result, output = self.run_helper("digest", reference=self.reference.split("@")[0] + "@sha256:" + "0" * 64)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(output.exists())
        self.script.unlink()
        self.script.symlink_to(self.helper)
        result, output = self.run_helper("symlink")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(output.exists())

    def test_real_container_failure_cleans_private_output(self):
        # Replace only the add-layer command with /bin/false in a real
        # container. This exercises the production failure cleanup after
        # private image allocation without changing the helper's interface.
        wrappers = self.root / "bin"
        wrappers.mkdir()
        real = shutil.which("podman")
        wrapper = wrappers / "podman"
        wrapper.write_text("#!/usr/bin/env python3\nimport os,sys\na=sys.argv[1:]\nif '/usr/bin/umoci' in a:\n i=a.index('--entrypoint'); a=a[:i]+['--entrypoint','/bin/false',a[i+2]]\nos.execv(" + repr(real) + ", ['podman']+a)\n")
        wrapper.chmod(0o755)
        env = dict(os.environ, PATH=str(wrappers) + os.pathsep + os.environ["PATH"])
        result, output = self.run_helper("failure", env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("returned non-zero exit status", result.stderr)
        self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
