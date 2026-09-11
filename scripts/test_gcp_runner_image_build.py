"""Mocked lifecycle tests for the immutable GCE runner image builder."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest


ROOT = Path(__file__).parent
FINGERPRINT = "a" * 64


FAKE_GCLOUD = r'''#!/usr/bin/env bash
set -Eeuo pipefail
state="${GCP_FAKE_STATE:?}"
mkdir -p "$state"
kind="$1 $2 $3"
case "$kind" in
  "compute images describe")
    if [[ -f "$state/image" ]]; then
      printf '{"name":"%s","status":"READY","labels":%s,"description":"%s"}\n' "$4" "$(cat "$state/image-labels")" "$(cat "$state/image-description")"
      exit 0
    fi
    printf "The resource 'projects/hephaestus-508000/global/images/%s' was not found\n" "$4" >&2
    exit 1
    ;;
  "compute instances describe")
    if [[ -f "$state/vm" ]]; then
      if [[ "${6:-}" == "--format=get(status)" || "${7:-}" == "--format=get(status)" ]]; then
        cat "$state/vm-status"
      else
        printf '{"labels":{"purpose":"hephaestus-runner-image-builder","run_id":"123","run_attempt":"1","repository_sha":"%s"}}\n' "$GITHUB_SHA"
      fi
      exit 0
    fi
    printf "The resource 'projects/hephaestus-508000/zones/europe-west1-d/instances/%s' was not found\n" "$4" >&2
    exit 1
    ;;
  "compute disks describe")
    if [[ -f "$state/disk" ]]; then
      printf '{"labels":%s}\n' "$(cat "$state/disk-labels")"
      exit 0
    fi
    printf "The resource 'projects/hephaestus-508000/zones/europe-west1-d/disks/%s' was not found\n" "$4" >&2
    exit 1
    ;;
  "compute disks create")
    touch "$state/disk"
    labels=''
    for argument in "$@"; do
      case "$argument" in --labels=*) labels="${argument#--labels=}" ;; esac
    done
    python3 - "$labels" <<'PYJSON' >"$state/disk-labels"
import json
import sys
print(json.dumps(dict(item.split("=", 1) for item in sys.argv[1].split(","))))
PYJSON
    printf 'create-disk\n' >>"$state/ops"
    exit 0
    ;;
  "compute instances create")
    touch "$state/vm"
    for argument in "$@"; do
      case "$argument" in
        --metadata=runner-image-fingerprint=*) printf '%s\n' "${argument#--metadata=runner-image-fingerprint=}" >"$state/fingerprint" ;;
        --disk=*) printf '%s\n' "${argument#--disk=}" >"$state/instance-disk" ;;
      esac
    done
    printf 'RUNNING\n' >"$state/vm-status"
    printf 'create-instance\n' >>"$state/ops"
    exit 0
    ;;
  "compute instances get-serial-port-output")
    if [[ "${GCP_FAKE_SERIAL:-ready}" == fail ]]; then
      printf 'HEPH_GCP_RUNNER_IMAGE: FAIL exit=17\n'
    else
      printf 'HEPH_GCP_RUNNER_IMAGE: READY fingerprint=%s\n' "$GCP_FAKE_FINGERPRINT"
    fi
    if [[ "${GCP_FAKE_SERIAL:-ready}" == hang ]]; then sleep 30; fi
    exit 0
    ;;
  "compute instances stop")
    printf 'TERMINATED\n' >"$state/vm-status"
    printf 'stop-instance\n' >>"$state/ops"
    exit 0
    ;;
  "compute instances delete")
    rm -f "$state/vm" "$state/vm-status"
    printf 'delete-instance\n' >>"$state/ops"
    exit 0
    ;;
  "compute images list")
    if [[ "${GCP_FAKE_ERROR:-}" == images-list ]]; then
      printf 'ERROR: image list permission was denied\n' >&2
      exit 13
    fi
    if [[ "${GCP_FAKE_WARNING:-}" == images-list ]]; then
      printf 'WARNING: a harmless list warning was emitted on stderr\n' >&2
    fi
    if [[ -f "$state/image" ]]; then
      printf '[{"name":"%s","status":"READY","labels":%s,"description":"%s"}]\n' "$(cat "$state/image-name")" "$(cat "$state/image-labels")" "$(cat "$state/image-description")"
    else
      printf '[]\n'
    fi
    exit 0
    ;;
  "compute images create")
    touch "$state/image"
    printf '%s\n' "$4" >"$state/image-name"
    labels=''
    for argument in "$@"; do
      case "$argument" in --labels=*) labels="${argument#--labels=}" ;; esac
    done
python3 - "$labels" <<'PYJSON' >"$state/image-labels"
import json
import sys
print(json.dumps(dict(item.split("=", 1) for item in sys.argv[1].split(","))))
PYJSON
    for argument in "$@"; do
      case "$argument" in --description=*) printf '%s' "${argument#--description=}" >"$state/image-description" ;; esac
    done
    printf 'create-image\n' >>"$state/ops"
    exit 0
    ;;
  "compute images delete")
    rm -f "$state/image"
    printf 'delete-image\n' >>"$state/ops"
    exit 0
    ;;
  "compute disks delete")
    rm -f "$state/disk"
    printf 'delete-disk\n' >>"$state/ops"
    exit 0
    ;;
esac
exit 2
'''


class RunnerImageBuildTests(unittest.TestCase):
    def _setup(self, root: Path) -> dict[str, str]:
        fake_bin = root / "bin"
        fake_bin.mkdir()
        fake = fake_bin / "gcloud"
        fake.write_text(FAKE_GCLOUD, encoding="utf-8")
        fake.chmod(0o700)
        manifest = root / "manifest.json"
        manifest.write_text(json.dumps({"fingerprint": FINGERPRINT}) + "\n", encoding="utf-8")
        startup = root / "startup.sh"
        startup.write_text("#!/usr/bin/env bash\n", encoding="utf-8")
        startup.chmod(0o700)
        return os.environ | {
            "PATH": f"{fake_bin}:{os.environ['PATH']}",
            "GCP_FAKE_STATE": str(root / "state"),
            "GCP_FAKE_FINGERPRINT": FINGERPRINT,
            "GCP_RUNNER_IMAGE_STATE": str(root / "runner-state"),
            "GITHUB_RUN_ID": "123",
            "GITHUB_RUN_ATTEMPT": "1",
            "GITHUB_SHA": "b" * 40,
            "GCP_ZONE": "europe-west1-d",
        }

    def test_fresh_build_deletes_only_owned_vm_and_disk_then_reruns_idempotently(self):
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-") as directory:
            root = Path(directory)
            env = self._setup(root)
            first = subprocess.run(
                [str(ROOT / "gcp-runner-image-build.sh"), "build"],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
            state = root / "state"
            self.assertTrue((state / "image").exists())
            self.assertFalse((state / "vm").exists())
            self.assertFalse((state / "disk").exists())
            operations = (state / "ops").read_text(encoding="utf-8").splitlines()
            self.assertEqual(operations, ["create-disk", "create-instance", "stop-instance", "delete-instance", "create-image", "delete-disk"])
            self.assertEqual((state / "instance-disk").read_text(encoding="utf-8").strip(),
                             "name=hephaestus-runner-disk-123-1-bbbbbbbbbbbb,boot=yes,auto-delete=no")
            self.assertIn('"purpose": "hephaestus-runner-image-builder"', (state / "disk-labels").read_text(encoding="utf-8"))
            second = subprocess.run(
                [str(ROOT / "gcp-runner-image-build.sh"), "build"],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
            self.assertEqual((state / "ops").read_text(encoding="utf-8").splitlines(), operations)

    def test_valid_json_stdout_survives_gcloud_warning_stderr(self):
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-warning-") as directory:
            root = Path(directory)
            env = self._setup(root) | {"GCP_FAKE_WARNING": "images-list"}
            result = subprocess.run(
                [str(ROOT / "gcp-runner-image-build.sh"), "build"],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("harmless list warning", result.stderr)
            self.assertFalse((root / "state" / "vm").exists())
            self.assertFalse((root / "state" / "disk").exists())

    def test_json_command_error_keeps_provider_stderr_visible(self):
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-error-") as directory:
            root = Path(directory)
            env = self._setup(root) | {"GCP_FAKE_ERROR": "images-list"}
            result = subprocess.run(
                [str(ROOT / "gcp-runner-image-build.sh"), "build"],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("image list permission was denied", result.stderr)
            self.assertNotIn("create-disk", (root / "state" / "ops").read_text(encoding="utf-8")
                              if (root / "state" / "ops").exists() else "")

    def test_provisioning_failure_cleans_owned_resources_and_candidate_image(self):
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-fail-") as directory:
            root = Path(directory)
            env = self._setup(root) | {"GCP_FAKE_SERIAL": "fail"}
            result = subprocess.run(
                [str(ROOT / "gcp-runner-image-build.sh"), "build"],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            state = root / "state"
            self.assertFalse((state / "image").exists())
            self.assertFalse((state / "vm").exists())
            self.assertFalse((state / "disk").exists())
            self.assertIn("delete-instance", (state / "ops").read_text(encoding="utf-8"))
            self.assertIn("delete-disk", (state / "ops").read_text(encoding="utf-8"))
            self.assertIn("HEPH_GCP_IMAGE_BUILD terminal=fail exit=17", result.stderr)

    def test_signal_cleanup_is_label_guarded_and_does_not_delete_unrelated_image(self):
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-signal-") as directory:
            root = Path(directory)
            env = self._setup(root) | {"GCP_FAKE_SERIAL": "hang"}
            process = subprocess.Popen(
                [str(ROOT / "gcp-runner-image-build.sh"), "build"],
                env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                start_new_session=True,
            )
            try:
                deadline = time.monotonic() + 3
                while not (root / "state" / "vm").exists() and time.monotonic() < deadline:
                    time.sleep(0.05)
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=8)
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                process.communicate()
            self.assertEqual(process.returncode, 143)
            state = root / "state"
            self.assertFalse((state / "vm").exists())
            self.assertFalse((state / "disk").exists())

    def test_cleanup_command_recovers_strict_building_state_and_owned_disk(self):
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-recover-") as directory:
            root = Path(directory)
            env = self._setup(root)
            state = root / "state"
            state.mkdir()
            (state / "vm").touch()
            (state / "vm-status").write_text("RUNNING\n", encoding="utf-8")
            (state / "disk").touch()
            (state / "disk-labels").write_text(
                '{"purpose":"hephaestus-runner-image-builder","run_id":"123",'
                '"run_attempt":"1","repository_sha":"' + "b" * 40 + '"}\n', encoding="utf-8"
            )
            (root / "runner-state").write_text(
                "schema=1\nstatus=building\nproject=hephaestus-508000\nzone=europe-west1-d\n"
                "run_id=123\nrun_attempt=1\nrepository_sha=" + "b" * 40 + "\n"
                "builder=hephaestus-runner-builder-123-1-bbbbbbbbbbbb\n"
                "disk=hephaestus-runner-disk-123-1-bbbbbbbbbbbb\nimage=\n"
                "fingerprint=\n", encoding="utf-8"
            )
            result = subprocess.run(
                [str(ROOT / "gcp-runner-image-build.sh"), "cleanup"],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse((state / "vm").exists())
            self.assertFalse((state / "disk").exists())

    def test_same_sha_image_from_older_run_is_reused_without_resources(self):
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-reuse-") as directory:
            root = Path(directory)
            env = self._setup(root)
            state = root / "state"
            state.mkdir()
            image_name = "hephaestus-runner-" + "a" * 32
            recipe = hashlib.sha256((ROOT / "gcp-runner-image-provision.sh").read_bytes()).hexdigest()
            verifier = hashlib.sha256((ROOT / "gcp-runner-image-verify.py").read_bytes()).hexdigest()
            startup = hashlib.sha256((ROOT / "gcp-kvm-startup.sh").read_bytes()).hexdigest()
            (state / "image").touch()
            (state / "image-name").write_text(image_name, encoding="utf-8")
            (state / "image-labels").write_text(json.dumps({
                "purpose": "hephaestus-runner-image", "run_id": "older-run",
                "run_attempt": "7", "repository_sha": "b" * 40, "fingerprint": "a" * 32,
            }), encoding="utf-8")
            (state / "image-description").write_text(
                "hephaestus-runner manifest_sha256=" + "a" * 64
                + " recipe_sha256=" + recipe + " verifier_sha256=" + verifier
                + " startup_sha256=" + startup, encoding="utf-8"
            )
            result = subprocess.run(
                [str(ROOT / "gcp-runner-image-build.sh"), "build"],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse((state / "ops").exists())

    def test_foreign_run_candidate_is_never_deleted_during_recovery(self):
        with tempfile.TemporaryDirectory(prefix="heph-runner-image-foreign-") as directory:
            root = Path(directory)
            env = self._setup(root)
            state = root / "state"
            state.mkdir()
            image_name = "hephaestus-runner-" + "a" * 32
            (state / "image").touch()
            (state / "image-name").write_text(image_name, encoding="utf-8")
            (state / "image-labels").write_text(json.dumps({
                "purpose": "hephaestus-runner-image", "run_id": "foreign",
                "run_attempt": "1", "repository_sha": "b" * 40, "fingerprint": "a" * 32,
            }), encoding="utf-8")
            (state / "image-description").write_text("foreign", encoding="utf-8")
            (root / "runner-state").write_text(
                "schema=1\nstatus=building\nproject=hephaestus-508000\nzone=europe-west1-d\n"
                "run_id=123\nrun_attempt=1\nrepository_sha=" + "b" * 40 + "\n"
                "builder=hephaestus-runner-builder-123-1-bbbbbbbbbbbb\n"
                "disk=hephaestus-runner-disk-123-1-bbbbbbbbbbbb\nimage=" + image_name + "\n"
                "fingerprint=" + "a" * 64 + "\n", encoding="utf-8"
            )
            result = subprocess.run(
                [str(ROOT / "gcp-runner-image-build.sh"), "cleanup"],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((state / "image").exists())
            self.assertIn("ownership labels do not match", result.stderr)

    def test_smoke_has_explicit_opt_in_custom_image_validation(self):
        smoke = (ROOT / "gcp-kvm-smoke.sh").read_text(encoding="utf-8")
        self.assertIn("GCP_RUNNER_IMAGE", smoke)
        self.assertIn("manifest.group(1)[:32]", smoke)
        self.assertIn("--image=\"$GCP_RUNNER_IMAGE\"", smoke)
        self.assertIn("--image-family=ubuntu-2404-lts-amd64", smoke)
        self.assertIn("json(name,status,labels,description)", smoke)
        self.assertIn("runner-image-manifest-sha256", smoke)
        self.assertIn("runner-image-recipe-sha256", smoke)
        self.assertIn("runner-image-verifier-sha256", smoke)
        self.assertIn("runner-image-startup-sha256", smoke)

    def test_workflow_keeps_image_build_manual_and_cleanup_separate(self):
        workflow = (ROOT.parent / ".github" / "workflows" / "cooking-e2e.yml").read_text(encoding="utf-8")
        self.assertIn("options: [preflight, image-build, image-retire, cache-preflight, diagnostics-triage, diagnostic, smoke, gcp-cooking, cooking]", workflow)
        self.assertIn("inputs.cloud_mode == 'image-build'", workflow)
        self.assertIn("gcp-runner-image-build.sh build", workflow)
        self.assertIn("gcp-runner-image-build.sh cleanup", workflow)
        self.assertIn("GCP_RUNNER_IMAGE: ${{ inputs.runner_image || vars.GCP_RUNNER_IMAGE }}", workflow)
        self.assertIn("GCP_USE_STOCK_IMAGE: ${{ inputs.use_stock_image }}", workflow)

    def test_generated_builder_wrapper_has_fail_closed_exit_marker(self):
        builder = (ROOT / "gcp-runner-image-build.sh").read_text(encoding="utf-8")
        self.assertIn('handle.write(\'finish() {\\n\')', builder)
        self.assertIn('HEPH_GCP_RUNNER_IMAGE: FAIL exit=%s', builder)


if __name__ == "__main__":
    unittest.main()
