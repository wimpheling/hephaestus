"""Functional coverage for the generated runner-image startup bundle."""
from __future__ import annotations

import base64
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).parent
BUILD = ROOT / "gcp-runner-image-build.sh"
BUNDLE_NAMES = (
    "gcp-runner-image-bake.sh",
    "gcp-runner-image-provision.sh",
    "gcp-runner-image-verify.py",
    "gcp-runner-image-manifest.py",
    "gcp-kvm-startup.sh",
    "gcp-passt-preflight.sh",
)


FAKE_GCLOUD = r'''#!/usr/bin/env bash
set -Eeuo pipefail
state="${GCP_FAKE_STATE:?}"
mkdir -p "$state"
case "$1 $2 $3" in
  "compute images list") printf '[]\n' ;;
  "compute images describe")
    printf "The resource 'projects/hephaestus-508000/global/images/%s' was not found\n" "$4" >&2
    exit 1
    ;;
  "compute disks create")
    touch "$state/disk"
    ;;
  "compute instances create")
    touch "$state/vm"
    for argument in "$@"; do
      case "$argument" in
        --metadata-from-file=startup-script=*) printf '%s\n' "${argument#--metadata-from-file=startup-script=}" >"${GCP_FAKE_CAPTURE:?}" ;;
      esac
    done
    ;;
  "compute instances get-serial-port-output")
    printf 'HEPH_GCP_RUNNER_IMAGE: FAIL exit=17\n'
    ;;
  "compute instances describe")
    if [[ -f "$state/vm" ]]; then
      printf '{"labels":{"purpose":"hephaestus-runner-image-builder","run_id":"123","run_attempt":"1","repository_sha":"%s"}}\n' "$GITHUB_SHA"
    else
      printf "The resource 'projects/hephaestus-508000/zones/europe-west1-d/resources/%s' was not found\n" "$4" >&2
      exit 1
    fi
    ;;
  "compute disks describe")
    if [[ -f "$state/disk" ]]; then
      printf '{"labels":{"purpose":"hephaestus-runner-image-builder","run_id":"123","run_attempt":"1","repository_sha":"%s"}}\n' "$GITHUB_SHA"
    else
      printf "The resource 'projects/hephaestus-508000/zones/europe-west1-d/resources/%s' was not found\n" "$4" >&2
      exit 1
    fi
    ;;
  "compute instances stop") printf 'TERMINATED\n' >"$state/status" ;;
  "compute instances delete") rm -f "$state/vm" "$state/status" ;;
  "compute disks delete") rm -f "$state/disk" ;;

  *) exit 2 ;;
esac
'''


class RunnerImageBundleTests(unittest.TestCase):
    def _environment(self, directory: Path) -> dict[str, str]:
        fake_bin = directory / "bin"
        fake_bin.mkdir()
        fake = fake_bin / "gcloud"
        fake.write_text(FAKE_GCLOUD, encoding="utf-8")
        fake.chmod(0o700)
        return os.environ | {
            "PATH": f"{fake_bin}:{os.environ['PATH']}",
            "GCP_FAKE_STATE": str(directory / "state"),
            "GCP_FAKE_CAPTURE": str(directory / "generated-startup.sh"),
            "GCP_RUNNER_IMAGE_STATE": str(directory / "runner-state"),
            "GITHUB_SHA": "b" * 40,
            "GITHUB_RUN_ID": "123",
            "GITHUB_RUN_ATTEMPT": "1",
            "GCP_ZONE": "europe-west1-d",
        }

    def _capture_bundle(self, directory: Path) -> tuple[Path, dict[str, str]]:
        environment = self._environment(directory)
        result = subprocess.run([str(BUILD), "build"], env=environment,
                                text=True, capture_output=True, check=False)
        self.assertNotEqual(result.returncode, 0)
        captured_path = directory / "generated-startup.sh"
        self.assertTrue(captured_path.is_file(), result.stdout + result.stderr)
        generated = Path(captured_path.read_text(encoding="utf-8").strip())
        self.assertTrue(generated.is_file(), result.stdout + result.stderr)
        local_copy = directory / "generated-startup-copy.sh"
        local_copy.write_bytes(generated.read_bytes())
        local_copy.chmod(0o700)
        generated.unlink()
        return local_copy, environment

    @staticmethod
    def _payload(wrapper: str) -> bytes:
        start_marker = "<<'HEPH_GCP_RUNNER_IMAGE_BUNDLE'\n"
        start = wrapper.index(start_marker) + len(start_marker)
        encoded, _ = wrapper[start:].split("\nHEPH_GCP_RUNNER_IMAGE_BUNDLE\n", 1)
        return base64.b64decode(encoded)

    @staticmethod
    def _wrapper_with_fixture(wrapper: str, fixture: bytes, names: tuple[str, ...]) -> str:
        source = io.BytesIO()
        with tarfile.open(fileobj=source, mode="w:gz") as archive:
            with tarfile.open(fileobj=io.BytesIO(RunnerImageBundleTests._payload(wrapper)), mode="r:gz") as original:
                for member in original.getmembers():
                    data = fixture if member.name == "gcp-runner-image-bake.sh" else original.extractfile(member).read()
                    member.size = len(data)
                    archive.addfile(member, io.BytesIO(data))
        encoded = base64.b64encode(source.getvalue()).decode("ascii")
        start_marker = "<<'HEPH_GCP_RUNNER_IMAGE_BUNDLE'\n"
        start = wrapper.index(start_marker) + len(start_marker)
        end = wrapper.index("\nHEPH_GCP_RUNNER_IMAGE_BUNDLE\n", start)
        lines = "\n".join(encoded[index:index + 120] for index in range(0, len(encoded), 120))
        return wrapper[:start] + lines + wrapper[end:]

    def test_generated_wrapper_executes_fixture_and_delivers_hashed_siblings(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-image-bundle-") as raw:
            directory = Path(raw)
            generated, environment = self._capture_bundle(directory)
            expected = {
                name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                for name in BUNDLE_NAMES
            }
            checks = "\n".join(
                (f'[[ "$(sha256sum "$fixture_dir/{name}" | awk \'{{print $1}}\')" == '
                 f'"$HEPH_GCP_EXPECTED_BAKE_SHA" ]]'
                 if name == "gcp-runner-image-bake.sh" else
                 f'[[ "$(sha256sum "$fixture_dir/{name}" | awk \'{{print $1}}\')" == "{digest}" ]]')
                for name, digest in expected.items()
            )
            fixture = ("#!/usr/bin/env bash\nset -Eeuo pipefail\n"
                       'fixture_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"\n'
                       f'[[ "${{HEPH_GCP_IMAGE_BAKE_REPO_SHA:-}}" == "{"b" * 40}" ]]\n'
                       f"{checks}\n"
                       "printf 'HEPH_GCP_TEST_BAKE: PASS\\n'\n").encode()
            wrapper = self._wrapper_with_fixture(generated.read_text(encoding="utf-8"), fixture, BUNDLE_NAMES)
            generated.write_text(wrapper, encoding="utf-8")
            temp_bin = directory / "wrapper-bin"
            temp_bin.mkdir()
            fake_mktemp = temp_bin / "mktemp"
            fake_mktemp.write_text(
                "#!/usr/bin/env bash\nset -Eeuo pipefail\n"
                f'if [[ "${{1:-}}" == /run/* || "${{2:-}}" == /run/* ]]; then exec /usr/bin/mktemp -d "{directory}/bundle.XXXXXX"; fi\n'
                "exec /usr/bin/mktemp \"$@\"\n", encoding="utf-8")
            fake_mktemp.chmod(0o700)
            result = subprocess.run([str(generated)], env=environment | {
                "PATH": f"{temp_bin}:{os.environ['PATH']}",
                "HEPH_GCP_EXPECTED_BAKE_SHA": hashlib.sha256(fixture).hexdigest()},
                text=True, capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("HEPH_GCP_TEST_BAKE: PASS", result.stdout)

    def test_missing_bundle_source_fails_before_mocked_create(self) -> None:
        missing = ROOT / "gcp-runner-image-manifest.py"
        backup = ROOT / ".gcp-runner-image-manifest.py.test-backup"
        self.assertFalse(backup.exists())
        missing.rename(backup)
        try:
            with tempfile.TemporaryDirectory(prefix="heph-image-bundle-missing-") as raw:
                directory = Path(raw)
                result = subprocess.run([str(BUILD), "build"], env=self._environment(directory),
                                        text=True, capture_output=True, check=False)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((directory / "generated-startup.sh").exists())
        finally:
            backup.rename(missing)


if __name__ == "__main__":
    unittest.main()
