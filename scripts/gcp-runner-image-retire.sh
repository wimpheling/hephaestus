#!/usr/bin/env bash
# Retire exactly one validated, project-owned immutable runner image.
set -Eeuo pipefail
umask 077

readonly PROJECT_ID='hephaestus-508000'
readonly IMAGE_PREFIX='hephaestus-runner'

die() { printf 'gcp-runner-image-retire: %s\n' "$*" >&2; exit 1; }

require_commands() {
  command -v gcloud >/dev/null 2>&1 || die 'gcloud is unavailable'
  command -v python3 >/dev/null 2>&1 || die 'python3 is unavailable'
}

is_not_found() {
  local image="$1" output="$2"
  grep -Eqi "images/${image}[^[:alnum:]_-].*(was not found|not found)" <<<"$output"
}

describe_image() {
  local image="$1" stderr_file output rc
  stderr_file="$(mktemp "${TMPDIR:-/tmp}/gcp-runner-image-retire-stderr.XXXXXX")"
  if output="$(gcloud compute images describe "$image" --project="$PROJECT_ID" --format=json 2>"$stderr_file")"; then
    rm -f -- "$stderr_file"
    printf '%s\n' "$output"
    return 0
  else
    rc=$?
  fi
  printf '%s\n' "$(cat -- "$stderr_file")" >&2
  rm -f -- "$stderr_file"
  return "$rc"
}

validate_image() {
  local image="$1" metadata="$2"
  IMAGE_PROJECT="$PROJECT_ID" RETIRE_IMAGE="$image" python3 - "$metadata" <<'PY'
import json
import os
import re
import sys

image = os.environ["RETIRE_IMAGE"]
project = os.environ["IMAGE_PROJECT"]
value = json.loads(sys.argv[1])
if not isinstance(value, dict):
    raise SystemExit("image metadata is not an object")
if value.get("name") != image:
    raise SystemExit("image metadata name does not match requested image")
if value.get("status") != "READY":
    raise SystemExit("image is not READY")
self_link = value.get("selfLink")
if not isinstance(self_link, str) or not re.fullmatch(
    rf"https://[A-Za-z0-9.-]+/compute/v1/projects/{re.escape(project)}/global/images/{re.escape(image)}",
    self_link,
):
    raise SystemExit("image is not in the expected Hephaestus project")
if "project" in value and value["project"] != project:
    raise SystemExit("image project metadata does not match expected project")
if not re.fullmatch(r"hephaestus-runner-[0-9a-f]{32}", image):
    raise SystemExit("image name is not an immutable Hephaestus runner image")
labels = value.get("labels")
if not isinstance(labels, dict) or labels.get("purpose") != "hephaestus-runner-image":
    raise SystemExit("image purpose label is not the immutable runner-image purpose")
fingerprint = labels.get("fingerprint")
if not isinstance(fingerprint, str) or not re.fullmatch(r"[0-9a-f]{32}", fingerprint):
    raise SystemExit("image fingerprint label is invalid")
if not re.fullmatch(r"[0-9a-f]{40}", str(labels.get("repository_sha", ""))):
    raise SystemExit("image repository provenance SHA is invalid")
description = value.get("description")
if not isinstance(description, str):
    raise SystemExit("image provenance description is missing")
anchors = {}
for field in ("manifest_sha256", "recipe_sha256", "verifier_sha256", "startup_sha256"):
    matches = re.findall(rf"(?:^|[ ,]){field}=([0-9a-f]{{64}})(?:$|[ ,])", description)
    if len(matches) != 1:
        raise SystemExit(f"image provenance {field} is missing or malformed")
    anchors[field] = matches[0]
if anchors["manifest_sha256"][:32] != fingerprint:
    raise SystemExit("image fingerprint does not match manifest provenance")
if image != "hephaestus-runner-" + anchors["manifest_sha256"][:32]:
    raise SystemExit("image name does not match manifest provenance")
print("validated immutable runner image " + image
      + " manifest_sha256=" + anchors["manifest_sha256"])
PY
}

verify_absent() {
  local image="$1" stderr_file output rc
  stderr_file="$(mktemp "${TMPDIR:-/tmp}/gcp-runner-image-retire-verify-stderr.XXXXXX")"
  if output="$(gcloud compute images describe "$image" --project="$PROJECT_ID" --format=json 2>"$stderr_file")"; then
    printf 'image remains present after retirement: %s\n%s\n' "$image" "$output" >&2
    rm -f -- "$stderr_file"
    return 1
  else
    rc=$?
  fi
  output="$(cat -- "$stderr_file")"
  rm -f -- "$stderr_file"
  if is_not_found "$image" "$output"; then
    printf 'retirement verified image absent: %s\n' "$image"
    return 0
  fi
  printf 'retirement verification was inconclusive for %s (exit=%s):\n%s\n' "$image" "$rc" "$output" >&2
  return 1
}

retire_image() {
  local image="$1" metadata delete_status=0
  [[ -n "$image" ]] || die 'an explicit runner image is required'
  [[ "$image" =~ ^hephaestus-runner-[0-9a-f]{32}$ ]] || die 'runner image name is invalid'
  [[ "$image" != "${GCP_PROTECTED_CURRENT_IMAGE:-}" ]] || die 'refusing to retire the configured current image'
  [[ "$image" != "${GCP_PROTECTED_ROLLBACK_IMAGE:-}" ]] || die 'refusing to retire the configured rollback image'
  [[ "$image" != "${GCP_RUNNER_IMAGE:-}" ]] || die 'refusing to retire the configured current image'
  [[ "$image" != "${GCP_RUNNER_ROLLBACK_IMAGE:-}" ]] || die 'refusing to retire the configured rollback image'

  if ! metadata="$(describe_image "$image")"; then
    die "cannot describe candidate image: $image"
  fi
  validate_image "$image" "$metadata" || die "candidate image failed immutable provenance validation: $image"

  if gcloud compute images delete "$image" --project="$PROJECT_ID" --quiet; then
    :
  else
    delete_status=$?
    printf 'image deletion command failed (exit=%s); independently checking absence\n' "$delete_status" >&2
  fi
  verify_absent "$image" || die "candidate image was not verified absent: $image"
  ((delete_status == 0)) || printf 'retirement accepted after verified absence despite deletion command status %s\n' "$delete_status" >&2
}

require_commands
[[ "${1:-}" == retire && "$#" == 2 ]] || die 'usage: scripts/gcp-runner-image-retire.sh retire IMAGE_NAME'
retire_image "$2"
